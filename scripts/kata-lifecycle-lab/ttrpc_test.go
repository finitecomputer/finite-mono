package main

// Pure Go regression at the actual helper transport seam. No socket, agent,
// process, engine or guest is used. PASS here means these bounds held.
import (
	"context"
	"encoding/binary"
	"errors"
	"io"
	"net"
	"testing"
	"time"
)

type observedWrite struct {
	net.Conn
	started chan struct{}
}

func (c *observedWrite) Write(b []byte) (int, error) {
	select {
	case c.started <- struct{}{}:
	default:
	}
	return c.Conn.Write(b)
}

func pipeClient(t *testing.T) (*ttrpcConn, net.Conn, <-chan struct{}) {
	t.Helper()
	a, b := net.Pipe()
	observed := &observedWrite{Conn: a, started: make(chan struct{}, 1)}
	c := newTTRPC("A", observed, nil)
	t.Cleanup(func() {
		c.close()
		b.Close()
		ctx, cancel := context.WithTimeout(context.Background(), time.Second)
		defer cancel()
		if err := c.drain(ctx); err != nil {
			t.Error(err)
		}
	})
	return c, b, observed.started
}

func resultWithin(t *testing.T, done <-chan rpcResult) rpcResult {
	t.Helper()
	select {
	case r := <-done:
		return r
	case <-time.After(300 * time.Millisecond):
		t.Fatal("call did not terminate within local bound")
		return rpcResult{}
	}
}

func TestWriteDeadlineControl(t *testing.T) {
	c, _, _ := pipeClient(t)
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Millisecond)
	defer cancel()
	done := make(chan rpcResult, 1)
	go func() { done <- c.call(ctx, "grpc.Health", "Check", nil, int64(20*time.Millisecond)) }()
	r := resultWithin(t, done)
	if r.err == nil || r.submission != "uncertain" {
		t.Fatalf("write deadline must preserve remote uncertainty: %+v", r)
	}
}

func TestCancellationDuringWrite(t *testing.T) {
	c, _, started := pipeClient(t)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	done := make(chan rpcResult, 1)
	go func() { done <- c.call(ctx, "grpc.AgentService", "WaitProcess", nil, 0) }()
	select {
	case <-started:
	case <-time.After(time.Second):
		t.Fatal("no submission attempt")
	}
	cancel()
	r := resultWithin(t, done)
	if !errors.Is(r.err, context.Canceled) || r.submission != "uncertain" {
		t.Fatalf("cancelled write: %+v", r)
	}
}

func TestDeadlineWhileWriterOwned(t *testing.T) {
	c, _, started := pipeClient(t)
	c.writer <- struct{}{}
	defer func() { <-c.writer }()
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Millisecond)
	defer cancel()
	done := make(chan rpcResult, 1)
	go func() { done <- c.call(ctx, "grpc.Health", "Check", nil, int64(20*time.Millisecond)) }()
	r := resultWithin(t, done)
	if !errors.Is(r.err, context.DeadlineExceeded) || r.submission != "not-sent" || r.stream != 0 {
		t.Fatalf("writer admission: %+v", r)
	}
	select {
	case <-started:
		t.Fatal("expired queued call wrote a frame")
	default:
	}
}

func TestCancelledContextNeverWrites(t *testing.T) {
	c, _, started := pipeClient(t)
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	r := c.call(ctx, "grpc.Health", "Check", nil, 0)
	if r.submission != "not-sent" || !errors.Is(r.err, context.Canceled) {
		t.Fatalf("pre-cancel: %+v", r)
	}
	select {
	case <-started:
		t.Fatal("cancelled context wrote")
	default:
	}
}

func TestPartialFrameClosesTransport(t *testing.T) {
	c, peer, _ := pipeClient(t)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	done := make(chan rpcResult, 1)
	go func() { done <- c.call(ctx, "grpc.AgentService", "WaitProcess", nil, 0) }()
	peer.SetReadDeadline(time.Now().Add(time.Second))
	if _, err := io.ReadFull(peer, make([]byte, 1)); err != nil {
		t.Fatal(err)
	}
	cancel()
	r := resultWithin(t, done)
	if r.submission != "uncertain" {
		t.Fatalf("partial write: %+v", r)
	}
	r = c.call(context.Background(), "grpc.Health", "Check", nil, 0)
	if r.err == nil || r.submission != "not-sent" {
		t.Fatalf("partial frame transport reused: %+v", r)
	}
}

func TestSubmittedCancellationRetainsLateReply(t *testing.T) {
	c, peer, _ := pipeClient(t)
	late := make(chan uint32, 1)
	c.onLate = func(_ string, stream uint32, method string, result rpcResult) {
		if method == "WaitProcess" && result.err == nil && result.code == 0 {
			late <- stream
		}
	}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	done := make(chan rpcResult, 1)
	go func() { done <- c.call(ctx, "grpc.AgentService", "WaitProcess", nil, 0) }()
	header := make([]byte, 10)
	peer.SetReadDeadline(time.Now().Add(time.Second))
	if _, err := io.ReadFull(peer, header); err != nil {
		t.Fatal(err)
	}
	if _, err := io.ReadFull(peer, make([]byte, binary.BigEndian.Uint32(header[:4]))); err != nil {
		t.Fatal(err)
	}
	// Full-frame receipt alone can race the sender's Write return. Establish
	// that writer ownership was released before testing response cancellation.
	select {
	case c.writer <- struct{}{}:
	case <-time.After(time.Second):
		t.Fatal("writer not released after full receipt")
	}
	cancel()
	<-c.writer
	r := resultWithin(t, done)
	if r.submission != "submitted" || !errors.Is(r.err, context.Canceled) {
		t.Fatalf("local cancellation collapsed accepted-work possibility: %+v", r)
	}
	header[8] = frameResponse
	binary.BigEndian.PutUint32(header[:4], 0)
	peer.SetWriteDeadline(time.Now().Add(time.Second))
	if _, err := peer.Write(header); err != nil {
		t.Fatal(err)
	}
	select {
	case stream := <-late:
		if stream != r.stream {
			t.Fatal("wrong late stream")
		}
	case <-time.After(time.Second):
		t.Fatal("late remote reply discarded")
	}
}

func TestWaitStatusNormalizationAndMalformedPayload(t *testing.T) {
	status, err := waitStatus(nil)
	if err != nil || status != 0 {
		t.Fatalf("proto3 zero omission: status=%d err=%v", status, err)
	}
	for _, payload := range [][]byte{{0x08}, pbBytes(nil, 1, []byte("bad"))} {
		if _, err := waitStatus(payload); err == nil {
			t.Fatalf("malformed WaitProcess accepted: %x", payload)
		}
	}
}

func TestMalformedEnvelopeCannotNormalizeToSuccess(t *testing.T) {
	for _, payload := range [][]byte{pbUint(nil, 1, 1), pbUint(nil, 2, 1), pbBytes(nil, 1, pbBytes(nil, 1, []byte("code"))),
		{0x08, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02}} {
		if _, _, _, err := ttrpcResponse(payload); err == nil {
			t.Fatalf("malformed response became success: %x", payload)
		}
	}
}

func TestHealthReadinessRequiresDecodedServing(t *testing.T) {
	for _, tc := range []struct {
		payload []byte
		want    string
	}{
		{pbUint(nil, 1, 1), "ok"}, {nil, "not-serving"}, {pbUint(nil, 1, 2), "not-serving"},
		{pbBytes(nil, 1, []byte("bad")), "malformed-response"},
	} {
		if name, _ := healthOutcome(rpcResult{payload: tc.payload}); name != tc.want {
			t.Fatalf("health=%x outcome=%s want=%s", tc.payload, name, tc.want)
		}
	}
}
