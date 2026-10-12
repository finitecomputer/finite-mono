package main

// In-memory local transport proof only. An intercepted deadline reset supplies
// a deterministic scheduling boundary after Write and AfterFunc detachment.
// The response is demonstrably queued before cancellation and the final select.
import (
	"context"
	"encoding/binary"
	"errors"
	"io"
	"net"
	"testing"
	"time"
)

type resetBarrier struct {
	net.Conn
	reached chan struct{}
	resume  chan struct{}
}

func (b *resetBarrier) SetWriteDeadline(t time.Time) error {
	if t.IsZero() {
		close(b.reached)
		<-b.resume
	}
	return b.Conn.SetWriteDeadline(t)
}

func oneReplyRace(t *testing.T, cancelBeforeSelect bool, forceTransfer ...bool) bool {
	t.Helper()
	local, peer := net.Pipe()
	barrier := &resetBarrier{Conn: local, reached: make(chan struct{}), resume: make(chan struct{})}
	late := make(chan rpcResult, 1)
	c := newTTRPC("A", barrier, func(_ string, _ uint32, _ string, result rpcResult) { late <- result })
	defer peer.Close()
	defer c.close()
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	done := make(chan rpcResult, 1)
	go func() { done <- c.call(ctx, "grpc.AgentService", "WaitProcess", nil, 0) }()
	peer.SetDeadline(time.Now().Add(time.Second))
	header := make([]byte, 10)
	if _, err := io.ReadFull(peer, header); err != nil {
		t.Fatal(err)
	}
	body := make([]byte, binary.BigEndian.Uint32(header[:4]))
	if _, err := io.ReadFull(peer, body); err != nil {
		t.Fatal(err)
	}
	select {
	case <-barrier.reached:
	case <-time.After(time.Second):
		t.Fatal("sender did not reach post-Write barrier")
	}
	stream := binary.BigEndian.Uint32(header[4:8])
	c.mu.Lock()
	responseQueue := c.wait[stream]
	c.mu.Unlock()
	if responseQueue == nil {
		t.Fatal("no pending stream before response")
	}
	response := pbBytes(nil, 2, pbUint(nil, 1, 7)) // successful remote wait, exit status 7
	binary.BigEndian.PutUint32(header[:4], uint32(len(response)))
	header[8] = frameResponse
	if _, err := peer.Write(append(header, response...)); err != nil {
		t.Fatal(err)
	}
	bound := time.Now().Add(time.Second)
	for len(responseQueue) != 1 && time.Now().Before(bound) {
		time.Sleep(time.Millisecond)
	}
	if len(responseQueue) != 1 {
		t.Fatal("response not queued at selected scheduling boundary")
	}
	if cancelBeforeSelect {
		cancel()
	}
	if len(forceTransfer) > 0 && forceTransfer[0] {
		// Exercise the cancellation handoff itself deterministically, while
		// the caller is still at the review's post-Write scheduling boundary.
		c.abandon(stream, callMeta{method: "WaitProcess", sentAt: time.Now()}, responseQueue)
		c.abandon(stream, callMeta{method: "WaitProcess", sentAt: time.Now()}, responseQueue)
		if len(late) != 1 || len(responseQueue) != 0 {
			t.Fatal("queued response was not transferred exactly once")
		}
	}
	close(barrier.resume)
	var result rpcResult
	select {
	case result = <-done:
	case <-time.After(time.Second):
		t.Fatal("call did not finish")
	}
	drainCtx, stop := context.WithTimeout(context.Background(), time.Second)
	defer stop()
	if err := c.drain(drainCtx); err != nil {
		t.Fatal(err)
	}
	if result.err == nil {
		status, err := waitStatus(result.payload)
		if err != nil || status != 7 {
			t.Fatalf("response corrupted: status=%d err=%v", status, err)
		}
		return false
	}
	if !cancelBeforeSelect || !errors.Is(result.err, context.Canceled) || result.submission != "submitted" {
		t.Fatalf("unexpected local result: %+v", result)
	}
	select {
	case reply := <-late:
		status, err := waitStatus(reply.payload)
		if err != nil || status != 7 || reply.code != 0 || reply.err != nil || reply.receivedAt.IsZero() {
			t.Fatalf("retained reply lost completion/status: %+v status=%d decode=%v", reply, status, err)
		}
		return false
	default:
	}
	c.mu.Lock()
	retainedWait, retainedLate := len(c.wait), len(c.gone)
	c.mu.Unlock()
	t.Logf("stream=%d returned cancellation; queued remote reply retained only in abandoned local channel=%d; wait=%d gone=%d late_callback=0", stream, len(responseQueue), retainedWait, retainedLate)
	return true
}

func TestReviewQueuedReplyControl(t *testing.T) { oneReplyRace(t, false) }

func TestCancellationHandoffTransfersQueuedReplyExactlyOnce(t *testing.T) {
	if oneReplyRace(t, true, true) {
		t.Fatal("queued response disappeared during explicit cancellation handoff")
	}
}

func TestReviewCancellationMustRetainAlreadyReceivedReply(t *testing.T) {
	lost := 0
	for i := 0; i < 16; i++ {
		if oneReplyRace(t, true) {
			lost++
		}
	}
	if lost != 0 {
		t.Fatalf("%d/16 completed remote replies disappeared at cancellation/select race", lost)
	}
}
