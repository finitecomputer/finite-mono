//go:build linux

package main

// A ttrpc client over one unix-socket connection: 10-byte big-endian frame
// header {length u32, stream u32, type u8, flags u8}, odd client stream IDs,
// unary requests only. Responses for calls the caller already abandoned are
// reported through onLate, never silently dropped.

import (
	"context"
	"encoding/binary"
	"errors"
	"fmt"
	"io"
	"net"
	"sync"
	"time"
)

const (
	frameRequest    = 1
	frameResponse   = 2
	maxFrame        = 4 << 20
	submissionBound = 5 * time.Second
)

type rpcResult struct {
	code       int64
	message    string
	payload    []byte
	err        error
	stream     uint32
	submission string // not-sent, uncertain, submitted; local close is never remote rollback
	sentAt     time.Time
	receivedAt time.Time // only a decoded wire response has a receipt timestamp
	requestID  uint64
}

type callMeta struct {
	method    string
	sentAt    time.Time
	requestID uint64
}

type ttrpcConn struct {
	name      string
	nc        net.Conn
	writer    chan struct{}
	mu        sync.Mutex
	next      uint32
	wait      map[uint32]chan rpcResult
	gone      map[uint32]callMeta
	closed    chan struct{}
	readDone  chan struct{}
	closeOnce sync.Once
	err       error
	onLate    func(conn string, stream uint32, method string, result rpcResult)
}

func dialTTRPC(name, path string, timeout time.Duration, onLate func(string, uint32, string, rpcResult)) (*ttrpcConn, error) {
	nc, err := net.DialTimeout("unix", path, timeout)
	if err != nil {
		return nil, err
	}
	return newTTRPC(name, nc, onLate), nil
}

func newTTRPC(name string, nc net.Conn, onLate func(string, uint32, string, rpcResult)) *ttrpcConn {
	c := &ttrpcConn{name: name, nc: nc, next: 1, wait: map[uint32]chan rpcResult{},
		gone: map[uint32]callMeta{}, writer: make(chan struct{}, 1), closed: make(chan struct{}),
		readDone: make(chan struct{}), onLate: onLate}
	go c.readLoop()
	return c
}

func (c *ttrpcConn) readLoop() {
	defer close(c.readDone)
	header := make([]byte, 10)
	var err error
	for {
		if _, err = io.ReadFull(c.nc, header); err != nil {
			break
		}
		size := binary.BigEndian.Uint32(header[0:4])
		stream := binary.BigEndian.Uint32(header[4:8])
		if size > maxFrame || header[8] != frameResponse {
			err = fmt.Errorf("invalid ttrpc frame type=%d size=%d", header[8], size)
			break
		}
		body := make([]byte, size)
		if _, err = io.ReadFull(c.nc, body); err != nil {
			break
		}
		code, message, payload, perr := ttrpcResponse(body)
		result := rpcResult{code: code, message: message, payload: payload, err: perr,
			stream: stream, submission: "submitted", receivedAt: time.Now()}
		c.mu.Lock()
		ch, waiting := c.wait[stream]
		delete(c.wait, stream)
		meta, abandoned := c.gone[stream]
		delete(c.gone, stream)
		// Enqueue under the same lock used by abandonment. The channel is
		// buffered and has exactly one unary result, so this never waits.
		if waiting {
			ch <- result
		}
		c.mu.Unlock()
		if abandoned && c.onLate != nil {
			result.sentAt, result.requestID = meta.sentAt, meta.requestID
			c.onLate(c.name, stream, meta.method, result)
		}
	}
	c.mu.Lock()
	c.err = err
	for stream, ch := range c.wait {
		ch <- rpcResult{err: fmt.Errorf("connection closed: %w", err)}
		delete(c.wait, stream)
	}
	c.mu.Unlock()
	c.close()
}

// call sends one request. timeoutNano is sent to the agent as the server-side
// deadline (0 = none), as Kata's host client does from its context deadline.
func (c *ttrpcConn) call(ctx context.Context, service, method string, payload []byte, timeoutNano int64) rpcResult {
	return c.callWithMeta(ctx, service, method, payload, timeoutNano, callMeta{sentAt: time.Now()})
}

func (c *ttrpcConn) callWithMeta(ctx context.Context, service, method string, payload []byte, timeoutNano int64, meta callMeta) rpcResult {
	meta.method = method
	// Writer admission is cancellable. No stream or remote operation exists yet.
	admission := time.NewTimer(submissionBound)
	defer admission.Stop()
	select {
	case c.writer <- struct{}{}:
	case <-ctx.Done():
		return rpcResult{err: ctx.Err(), submission: "not-sent"}
	case <-c.closed:
		return rpcResult{err: errors.New("connection closed"), submission: "not-sent"}
	case <-admission.C:
		return rpcResult{err: context.DeadlineExceeded, submission: "not-sent"}
	}
	if err := ctx.Err(); err != nil {
		<-c.writer
		return rpcResult{err: err, submission: "not-sent"}
	}
	request := ttrpcRequest(service, method, payload, timeoutNano)
	ch := make(chan rpcResult, 1)
	c.mu.Lock()
	if c.err != nil {
		c.mu.Unlock()
		<-c.writer
		return rpcResult{err: errors.New("connection closed"), submission: "not-sent"}
	}
	stream := c.next
	c.next += 2
	c.wait[stream] = ch
	c.mu.Unlock()

	frame := make([]byte, 10, 10+len(request))
	binary.BigEndian.PutUint32(frame[0:4], uint32(len(request)))
	binary.BigEndian.PutUint32(frame[4:8], stream)
	frame[8] = frameRequest
	frame = append(frame, request...)
	// A finite helper submission bound also covers WaitProcess's deadline-less
	// response context. This is a lab transport bound, not an agent stop policy.
	deadline := time.Now().Add(submissionBound)
	if d, ok := ctx.Deadline(); ok && d.Before(deadline) {
		deadline = d
	}
	err := c.nc.SetWriteDeadline(deadline)
	if err != nil {
		c.close()
		c.abandon(stream, meta, ch)
		<-c.writer
		return rpcResult{err: err, stream: stream, submission: "not-sent"}
	}
	callbackDone := make(chan struct{})
	stopCancellation := context.AfterFunc(ctx, func() {
		// Closing makes a partial frame unusable by the next sender and wakes
		// reader/waiters. Accepted remote work remains unresolved.
		c.close()
		close(callbackDone)
	})
	n, err := c.nc.Write(frame)
	if !stopCancellation() {
		<-callbackDone
	}
	if err == nil && n != len(frame) {
		err = io.ErrShortWrite
	}
	if err == nil {
		err = c.nc.SetWriteDeadline(time.Time{})
	}
	if err != nil {
		c.close()
	}
	<-c.writer
	if err != nil {
		c.abandon(stream, meta, ch)
		if ctx.Err() != nil {
			err = ctx.Err()
		}
		return rpcResult{err: err, stream: stream, submission: "uncertain"}
	}
	select {
	case r := <-ch:
		r.stream, r.submission = stream, "submitted"
		r.sentAt, r.requestID = meta.sentAt, meta.requestID
		return r
	case <-ctx.Done():
		c.abandon(stream, meta, ch)
		return rpcResult{err: ctx.Err(), stream: stream, submission: "submitted"}
	}
}

func (c *ttrpcConn) abandon(stream uint32, meta callMeta, ch chan rpcResult) {
	c.mu.Lock()
	if _, ok := c.wait[stream]; ok {
		delete(c.wait, stream)
		c.gone[stream] = meta
		c.mu.Unlock()
		return
	}
	// The reader may already have delivered before cancellation won select.
	// Transfer that response instead of stranding it in the returned call's
	// channel. Connection-close notifications have no remote receipt time.
	var result rpcResult
	select {
	case result = <-ch:
	default:
	}
	c.mu.Unlock()
	if !result.receivedAt.IsZero() && c.onLate != nil {
		result.sentAt, result.requestID = meta.sentAt, meta.requestID
		c.onLate(c.name, stream, meta.method, result)
	}
}

func (c *ttrpcConn) close() {
	c.closeOnce.Do(func() {
		close(c.closed)
		c.nc.Close()
	})
}

func (c *ttrpcConn) drain(ctx context.Context) error {
	c.close()
	select {
	case <-c.readDone:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}

func (r rpcResult) fields(method string) map[string]any {
	name, detail := outcome(r)
	fields := map[string]any{"outcome": name, "detail": detail, "method": method,
		"stream": r.stream, "code": r.code, "submission": r.submission}
	if !r.sentAt.IsZero() {
		fields["sent_t"] = r.sentAt.UTC().Format(time.RFC3339Nano)
	}
	if !r.receivedAt.IsZero() {
		fields["received_t"] = r.receivedAt.UTC().Format(time.RFC3339Nano)
	}
	if r.requestID != 0 {
		fields["request_id"] = r.requestID
	}
	return fields
}

func healthOutcome(r rpcResult) (string, string) {
	name, detail := outcome(r)
	if name != "ok" {
		return name, detail
	}
	fields, err := pbParse(r.payload)
	if err != nil {
		return "malformed-response", err.Error()
	}
	status := fields[1]
	if len(status) > 0 && status[len(status)-1].wire != 0 {
		return "malformed-response", "health status wire type"
	}
	if lastUint(fields, 1) != 1 {
		return "not-serving", "HealthCheckResponse.status != SERVING"
	}
	return "ok", ""
}

func waitStatus(payload []byte) (int32, error) {
	fields, err := pbParse(payload)
	if err != nil {
		return 0, err
	}
	if status := fields[1]; len(status) > 0 && status[len(status)-1].wire != 0 {
		return 0, errors.New("wait status wire type")
	}
	// Proto3 legitimately omits status=0. The normalized successful record
	// still contains an explicit decoded status, including that zero.
	return int32(lastUint(fields, 1)), nil
}

// outcome names an RPC result for records: ok, deadline, rpc-error or transport-error.
func outcome(r rpcResult) (string, string) {
	switch {
	case r.err == nil && r.code == 0:
		return "ok", ""
	case r.err == nil && r.code == 4:
		return "deadline", r.message
	case r.err == nil:
		return "rpc-error", fmt.Sprintf("code=%d %s", r.code, r.message)
	case errors.Is(r.err, context.DeadlineExceeded):
		return "deadline", r.err.Error()
	case errors.Is(r.err, context.Canceled):
		return "cancelled", r.err.Error()
	default:
		return "transport-error", r.err.Error()
	}
}
