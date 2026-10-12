//go:build linux

package main

// One agent stop arm. Connection A plays the shim's single long-lived
// connection: setup, an outstanding WaitProcess for init, SignalProcess
// (exec_id "") and a monitor-style Check (5 s ticker, 30 s deadline).
// Connection B is an independent 1 s Check loop. A sampler records the
// synthetic cgroup, task states and unit FreezerState. The client records
// outcomes; it does not judge them. Exit 0 means the protocol ran to its end.

import (
	"context"
	"encoding/json"
	"flag"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"sync"
	"sync/atomic"
	"time"
)

const (
	exitSetup        = 10
	exitPrecondition = 11
)

type armClient struct {
	rec     *recorder
	sigAt   time.Time
	sigMu   sync.Mutex
	pidsMu  sync.Mutex
	seen    map[int]bool
	agentID int
	ctx     context.Context
	ended   atomic.Bool
	checks  atomic.Uint64
}

func (a *armClient) relSignal(t time.Time) any {
	a.sigMu.Lock()
	defer a.sigMu.Unlock()
	if a.sigAt.IsZero() {
		return nil
	}
	return t.Sub(a.sigAt).Seconds()
}

func (a *armClient) check(conn *ttrpcConn, timeout time.Duration) bool {
	sent := time.Now()
	requestID := a.checks.Add(1)
	a.rec.emit("check-started", map[string]any{"conn": conn.name, "request_id": requestID,
		"sent_t": sent.UTC().Format(time.RFC3339Nano)})
	ctx, cancel := context.WithTimeout(a.ctx, timeout)
	r := conn.callWithMeta(ctx, "grpc.Health", "Check", nil, int64(timeout), callMeta{sentAt: sent, requestID: requestID})
	cancel()
	name, detail := healthOutcome(r)
	fields := r.fields("Check")
	fields["conn"], fields["sent_t"] = conn.name, sent.UTC().Format(time.RFC3339Nano)
	fields["sent_rel_signal_s"], fields["latency_ms"] = a.relSignal(sent), ms(time.Since(sent))
	fields["deadline_ms"], fields["outcome"], fields["detail"] = ms(timeout), name, detail
	fields["request_id"] = requestID
	if name == "ok" {
		fields["health_status"] = 1
	}
	kind := "check"
	if a.ended.Load() {
		kind = "check-abandoned"
	}
	a.rec.emit(kind, fields)
	return name == "ok" && r.submission == "submitted"
}

func (a *armClient) sample(dir, unit string) {
	fields := map[string]any{"cgroup_state": cgroupState(dir)}
	if procs, ok := fields["cgroup_state"].(map[string]any)["all_procs"].([]int); ok {
		a.pidsMu.Lock()
		for _, pid := range procs {
			a.seen[pid] = true
		}
		a.pidsMu.Unlock()
	}
	a.pidsMu.Lock()
	tasks := map[string]string{}
	for pid := range a.seen {
		tasks[strconv.Itoa(pid)] = taskState(pid)
	}
	a.pidsMu.Unlock()
	fields["tasks"] = tasks
	if unit != "" {
		fields["unit"] = unitState(unit)
	}
	if a.agentID > 0 {
		workers, inEpoll := threadSummary(a.agentID)
		fields["agent_workers"], fields["agent_workers_in_epoll"] = workers, inEpoll
	}
	fields["rel_signal_s"] = a.relSignal(time.Now())
	a.rec.emit("sample", fields)
}

func loop(stop <-chan struct{}, wg *sync.WaitGroup, every time.Duration, f func()) {
	wg.Add(1)
	go func() {
		defer wg.Done()
		t := time.NewTicker(every)
		defer t.Stop()
		for {
			select {
			case <-stop:
				return
			case <-t.C:
				f()
			}
		}
	}()
}

type heldRequest struct {
	Pid    int    `json:"pid"`
	Unique uint64 `json:"unique"`
}

func clientMain(args []string) {
	fs := flag.NewFlagSet("client", flag.ExitOnError)
	run := fs.String("run", "", "run generation")
	sock := fs.String("sock", "/run/fin175/agent.sock", "agent unix socket")
	cid := fs.String("cid", "", "synthetic container (and sandbox) id")
	mode := fs.String("cgroups", "systemd", "systemd => system.slice:nerdctl:<cid>; fs => /finlab/<cid>")
	reader := fs.Bool("reader", false, "start a reader held in a FUSE READ")
	initOnTerm := fs.String("init-on-term", "exit", "init reaction to SIGTERM: exit|stay")
	signal := fs.Uint("signal", 15, "SignalProcess signal number")
	state := fs.String("state", "/run/finlab/state", "fused state directory")
	agentPid := fs.Int("agent-pid", 0, "agent pid from identity, for thread samples")
	sigDeadline := fs.Duration("signal-deadline", 60*time.Second, "SignalProcess deadline (Kata defaultRequestTimeout)")
	window := fs.Duration("window", 20*time.Second, "after SignalProcess returns, wait this long for init exit")
	tail := fs.Duration("tail", 2*time.Second, "observation after the arm's last action")
	maxRun := fs.Duration("max", 120*time.Second, "hard bound from SignalProcess to client end")
	minObserve := fs.Duration("min-observe", 12*time.Second, "observe at least this long after SignalProcess")
	fs.Parse(args)
	if !cidPattern.MatchString(*cid) || (*mode != "systemd" && *mode != "fs") ||
		(*initOnTerm != "exit" && *initOnTerm != "stay") || *agentPid <= 0 || *signal == 0 || *signal > 64 ||
		*sigDeadline <= 0 || *window <= 0 || *maxRun <= 0 || *tail < 0 || *minObserve <= 0 {
		fatalf(2, "invalid -cid, -cgroups or -init-on-term")
	}
	rec := newRecorder(os.Stdout, *run, "client")
	rec.bound = map[string]any{"cid": *cid, "operation": *run + ":stop:" + *cid}
	a := &armClient{rec: rec, seen: map[int]bool{}, agentID: *agentPid, ctx: context.Background()}
	onLate := func(conn string, stream uint32, method string, result rpcResult) {
		fields := result.fields(method)
		fields["conn"], fields["latency_ms"] = conn, ms(result.receivedAt.Sub(result.sentAt))
		fields["rel_signal_s"] = a.relSignal(time.Now())
		if method == "Check" {
			name, detail := healthOutcome(result)
			fields["outcome"], fields["detail"] = name, detail
			if name == "ok" {
				fields["health_status"] = 1
			}
		}
		if method == "WaitProcess" {
			fields["exec_id"] = *cid
			if name, _ := outcome(result); name == "ok" {
				if status, err := waitStatus(result.payload); err == nil {
					fields["status"] = status
				} else {
					fields["outcome"], fields["detail"] = "malformed-response", err.Error()
				}
			}
		}
		if method == "SignalProcess" {
			fields["exec_id"] = ""
		}
		rec.emit("late-response", fields)
	}
	connA, err := dialTTRPC("A", *sock, 5*time.Second, onLate)
	if err != nil {
		rec.emit("setup-error", map[string]any{"step": "dial-A", "detail": err.Error()})
		os.Exit(exitSetup)
	}
	defer connA.close()
	connB, err := dialTTRPC("B", *sock, 5*time.Second, onLate)
	if err != nil {
		rec.emit("setup-error", map[string]any{"step": "dial-B", "detail": err.Error()})
		os.Exit(exitSetup)
	}
	defer connB.close()
	readyA := a.check(connA, 5*time.Second)
	readyB := a.check(connB, 5*time.Second)
	if !readyA || !readyB {
		rec.emit("precondition-error", map[string]any{"step": "readiness", "detail": "both connections must be SERVING before workload RPCs"})
		connA.close()
		connB.close()
		os.Exit(exitPrecondition)
	}

	setup := func(step, method string, payload []byte, timeout time.Duration) {
		sent := time.Now()
		ctx, cancel := context.WithTimeout(context.Background(), timeout)
		r := connA.callWithMeta(ctx, "grpc.AgentService", method, payload, int64(timeout), callMeta{sentAt: sent})
		cancel()
		if name, detail := outcome(r); name != "ok" {
			rec.emit("setup-error", map[string]any{"step": step, "outcome": name, "detail": detail})
			os.Exit(exitSetup)
		}
		fields := r.fields(method)
		fields["step"], fields["conn"], fields["latency_ms"] = step, "A", ms(time.Since(sent))
		rec.emit("setup-ok", fields)
	}
	setup("create-sandbox", "CreateSandbox", createSandboxRequest("finlab", *cid), 30*time.Second)
	env := []string{"PATH=/", "FINLAB_RUN=" + *run, "FINLAB_INIT_ON_TERM=" + *initOnTerm}
	if *reader {
		env = append(env, "FINLAB_READER=/fuse/hold")
	}
	spec := ociSpec(syntheticSpec{
		rootfs: "/var/lib/finlab/rootfs", cgroupsPath: cgroupsPath(*mode, *cid),
		args: []string{"/finlab", "init"}, env: env, memoryLimit: 1 << 30, cpuQuota: 200000, cpuPeriod: 100000,
		mounts: []mountSpec{
			{"/proc", "proc", "proc", []string{"nosuid", "noexec", "nodev"}},
			{"/dev", "tmpfs", "tmpfs", []string{"nosuid", "strictatime", "mode=755", "size=65536k"}},
			{"/sys", "sysfs", "sysfs", []string{"nosuid", "noexec", "nodev", "ro"}},
			{"/fuse", "bind", "/run/finlab/mnt", []string{"rbind", "rw"}},
			{"/log", "bind", "/run/finlab/log", []string{"rbind", "rw"}},
		}})
	setup("create-container", "CreateContainer", createContainerRequest(*cid, *cid, spec), 60*time.Second)
	rec.emit("container-created", map[string]any{"cid": *cid, "cgroups_path": cgroupsPath(*mode, *cid)})
	setup("start-container", "StartContainer", containerRequest(*cid), 30*time.Second)

	dir := cgroupDir(*mode, *cid)
	unit := ""
	if *mode == "systemd" {
		unit = "nerdctl-" + *cid + ".scope"
	}
	runCtx, stopRun := context.WithCancel(context.Background())
	defer stopRun()
	a.ctx = runCtx
	// The kind marks local shutdown; the fields retain actual outcomes and
	// receipt timestamps. It never implies remote abort or erases a Check.
	resultKind := func(kind string) string {
		if a.ended.Load() {
			return kind + "-abandoned"
		}
		return kind
	}
	waitDone := make(chan struct{})
	go func() {
		defer close(waitDone)
		start := time.Now()
		rec.emit("wait-sent", map[string]any{"exec_id": *cid, "conn": "A"})
		r := connA.callWithMeta(runCtx, "grpc.AgentService", "WaitProcess", waitProcessRequest(*cid, *cid), 0, callMeta{sentAt: start})
		name, _ := outcome(r)
		fields := r.fields("WaitProcess")
		fields["conn"], fields["exec_id"] = "A", *cid
		fields["sent_t"] = start.UTC().Format(time.RFC3339Nano)
		fields["latency_ms"], fields["rel_signal_s"] = ms(time.Since(start)), a.relSignal(time.Now())
		if name == "ok" {
			status, err := waitStatus(r.payload)
			if err != nil {
				fields["outcome"], fields["detail"] = "malformed-response", err.Error()
			} else {
				fields["status"] = status
			}
		}
		rec.emit(resultKind("wait-returned"), fields)
	}()

	if *reader {
		marker := filepath.Join(*state, "read-pending")
		var held heldRequest
		deadline := time.Now().Add(20 * time.Second)
		for {
			if b, err := os.ReadFile(marker); err == nil && json.Unmarshal(b, &held) == nil && held.Pid > 0 && held.Unique > 0 {
				break
			}
			if time.Now().After(deadline) {
				rec.emit("precondition-error", map[string]any{"step": "held-request", "detail": "no FUSE READ received"})
				os.Exit(exitPrecondition)
			}
			time.Sleep(50 * time.Millisecond)
		}
		deadline = time.Now().Add(5 * time.Second)
		for {
			procs, _ := cgroupState(dir)["procs"].([]int)
			if containsPid(procs, held.Pid) {
				break
			}
			if time.Now().After(deadline) {
				rec.emit("precondition-error", map[string]any{"step": "held-request-cgroup", "pid": held.Pid})
				os.Exit(exitPrecondition)
			}
			time.Sleep(50 * time.Millisecond)
		}
		rec.emit("held-request", map[string]any{"pid": held.Pid, "unique": held.Unique, "task": taskState(held.Pid)})
	}

	stop := make(chan struct{})
	var loops sync.WaitGroup
	loop(stop, &loops, 5*time.Second, func() { a.check(connA, 30*time.Second) })
	loop(stop, &loops, time.Second, func() { a.check(connB, 5*time.Second) })
	loop(stop, &loops, 500*time.Millisecond, func() { a.sample(dir, unit) })
	time.Sleep(2 * time.Second)
	a.sample(dir, unit)

	a.sigMu.Lock()
	a.sigAt = time.Now()
	a.sigMu.Unlock()
	rec.emit("signal-sent", map[string]any{"signal": *signal, "exec_id": "", "rel_signal_s": 0.0})
	sigDone := make(chan struct{})
	go func() {
		defer close(sigDone)
		ctx, cancel := context.WithTimeout(runCtx, *sigDeadline)
		r := connA.callWithMeta(ctx, "grpc.AgentService", "SignalProcess", signalProcessRequest(*cid, "", uint32(*signal)), int64(*sigDeadline), callMeta{sentAt: a.sigAt})
		cancel()
		fields := r.fields("SignalProcess")
		fields["conn"], fields["exec_id"] = "A", ""
		fields["sent_t"] = a.sigAt.UTC().Format(time.RFC3339Nano)
		fields["latency_ms"], fields["rel_signal_s"] = ms(time.Since(a.sigAt)), a.relSignal(time.Now())
		rec.emit(resultKind("signal-returned"), fields)
	}()

	runTimer := time.AfterFunc(*maxRun, stopRun)
	defer runTimer.Stop()
	hardStop := runCtx.Done()
	reason := "complete"
	select {
	case <-sigDone:
		select {
		case <-waitDone:
			a.pidsMu.Lock()
			pids := []int{}
			for pid := range a.seen {
				pids = append(pids, pid)
			}
			a.pidsMu.Unlock()
			rec.emit("terminal-observation", snapshot(runCtx, *cid, *mode, *agentPid, pids, connB))
		case <-time.After(*window):
			reason = "window-elapsed"
		case <-hardStop:
			reason = "max-reached"
		}
	case <-hardStop:
		reason = "max-reached"
	}
	if reason != "max-reached" {
		if remaining := *minObserve - time.Since(a.sigAt); remaining > 0 {
			select {
			case <-time.After(remaining):
			case <-hardStop:
				reason = "max-reached"
			}
		}
		select {
		case <-time.After(*tail):
		case <-hardStop:
			reason = "max-reached"
		}
	}
	close(stop)
	a.ended.Store(true)
	pending := []string{}
	select {
	case <-sigDone:
	default:
		pending = append(pending, "signal")
	}
	select {
	case <-waitDone:
	default:
		pending = append(pending, "wait")
	}
	stopRun()
	connA.close()
	connB.close()
	loopsDone := make(chan struct{})
	go func() { loops.Wait(); close(loopsDone) }()
	select {
	case <-loopsDone:
	case <-time.After(3 * time.Second):
		rec.emit("shutdown-error", map[string]any{"detail": "observation loops did not drain"})
		os.Exit(exitSetup)
	}
	for _, done := range []chan struct{}{sigDone, waitDone} {
		select {
		case <-done:
		case <-time.After(2 * time.Second):
			rec.emit("shutdown-error", map[string]any{"detail": "RPC sender did not drain"})
			os.Exit(exitSetup)
		}
	}
	for _, conn := range []*ttrpcConn{connA, connB} {
		ctx, cancel := context.WithTimeout(context.Background(), time.Second)
		err := conn.drain(ctx)
		cancel()
		if err != nil {
			rec.emit("shutdown-error", map[string]any{"detail": err.Error()})
			os.Exit(exitSetup)
		}
	}
	a.pidsMu.Lock()
	seen := []int{}
	for pid := range a.seen {
		seen = append(seen, pid)
	}
	a.pidsMu.Unlock()
	sort.Ints(seen)
	rec.emit("client-end", map[string]any{"reason": reason, "pending": pending, "seen_pids": seen})
}

func containsPid(pids []int, pid int) bool {
	for _, p := range pids {
		if p == pid {
			return true
		}
	}
	return false
}
