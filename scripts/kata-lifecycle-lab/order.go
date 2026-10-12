//go:build linux

package main

// Real systemd FreezeUnit/ThawUnit primitives on a synthetic scope whose only
// task is held in a FUSE READ, so a freeze stays pending. Ordering is
// established by observed state (cgroup.freeze, FreezerState) and by command
// completion, never by sleeping. busctl (sd-bus) is the client; this does not
// exercise the kata-agent or zbus.
//
//	thaw-cancels-accepted-freeze  a ThawUnit sent after the freeze was observed
//	                              accepted cancels it (FreezeCancelled)
//	thaw-before-freeze            a completed ThawUnit does not fence a later
//	                              FreezeUnit (expected negative primitive)
//	local-timeout-not-rollback    a FreezeUnit whose client gave up stays
//	                              pending in systemd

import (
	"bytes"
	"context"
	"encoding/json"
	"flag"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"
)

type orderProbe struct {
	rec   *recorder
	run   string
	state string
	exe   string
}

type busResult struct {
	exit   int
	stderr string
	stdout string
	timed  bool
}

func busctl(timeout time.Duration, method, unit string, callTimeout time.Duration) busResult {
	ctx, cancel := context.WithTimeout(context.Background(), timeout)
	defer cancel()
	cmd := exec.CommandContext(ctx, "busctl", "call", "--timeout="+callTimeout.String(),
		"org.freedesktop.systemd1", "/org/freedesktop/systemd1", "org.freedesktop.systemd1.Manager", method, "s", unit)
	cmd.WaitDelay = 250 * time.Millisecond
	var out, errb bytes.Buffer
	cmd.Stdout, cmd.Stderr = &out, &errb
	err := cmd.Run()
	r := busResult{stdout: strings.TrimSpace(out.String()), stderr: strings.TrimSpace(errb.String()), timed: ctx.Err() != nil}
	if err != nil {
		r.exit = -1
		if ee, ok := err.(*exec.ExitError); ok {
			r.exit = ee.ExitCode()
		}
	}
	return r
}

func (r busResult) fields() map[string]any {
	return map[string]any{"exit": r.exit, "stderr": r.stderr, "stdout": r.stdout, "bound_hit": r.timed}
}

// background starts a FreezeUnit call and returns a channel with its result.
func backgroundBusctl(timeout time.Duration, method, unit string, callTimeout time.Duration) <-chan busResult {
	done := make(chan busResult, 1)
	go func() { done <- busctl(timeout, method, unit, callTimeout) }()
	return done
}

func (p *orderProbe) unitObservation(unit string) map[string]any {
	dir := "/sys/fs/cgroup/system.slice/" + unit
	return map[string]any{"cgroup_state": cgroupState(dir), "unit": unitState(unit)}
}

func observedFreezing(obs map[string]any) bool {
	cg, _ := obs["cgroup_state"].(map[string]any)
	unit, _ := obs["unit"].(map[string]any)
	return cg["freeze"] == "1" && cg["frozen"] == "0" && unit["FreezerState"] == "freezing"
}

func observedRunning(obs map[string]any) bool {
	cg, _ := obs["cgroup_state"].(map[string]any)
	unit, _ := obs["unit"].(map[string]any)
	return cg["freeze"] == "0" && cg["frozen"] == "0" && unit["FreezerState"] == "running"
}

// waitFor polls an observation until pred holds or the bound passes.
func (p *orderProbe) waitFor(unit string, bound time.Duration, pred func(map[string]any) bool) (map[string]any, bool) {
	deadline := time.Now().Add(bound)
	for {
		obs := p.unitObservation(unit)
		if pred(obs) {
			return obs, true
		}
		if time.Now().After(deadline) {
			return obs, false
		}
		time.Sleep(50 * time.Millisecond)
	}
}

// heldScope starts a transient scope whose only task blocks in a FUSE READ.
func (p *orderProbe) heldScope(caseName string) (string, bool) {
	unit := "finlab-" + p.run[:12] + "-" + caseName + ".scope"
	cmd := exec.Command("systemd-run", "--scope", "--quiet", "--unit="+unit, "--", p.exe, "read", "/run/finlab/mnt/hold")
	cmd.Env = append(os.Environ(), "FINLAB_RUN="+p.run, "FINLAB_LOG=/run/finlab/log/order-readers.jsonl")
	if err := cmd.Start(); err != nil {
		p.rec.emit("case-setup-error", map[string]any{"case": caseName, "detail": err.Error()})
		return unit, false
	}
	go cmd.Wait()
	deadline := time.Now().Add(15 * time.Second)
	for time.Now().Before(deadline) {
		var held heldRequest
		b, err := os.ReadFile(filepath.Join(p.state, "read-pending"))
		procs, _ := cgroupState("/sys/fs/cgroup/system.slice/" + unit)["procs"].([]int)
		if err == nil && json.Unmarshal(b, &held) == nil && containsPid(procs, held.Pid) {
			p.rec.emit("held-request", map[string]any{"case": caseName, "unit": unit, "pid": held.Pid,
				"unique": held.Unique, "task": taskState(held.Pid)})
			return unit, true
		}
		time.Sleep(50 * time.Millisecond)
	}
	p.rec.emit("case-setup-error", map[string]any{"case": caseName, "detail": "reader never held in scope"})
	return unit, false
}

func (p *orderProbe) emitStep(caseName, step string, fields map[string]any) {
	fields["case"], fields["step"] = caseName, step
	fields["unit"] = "finlab-" + p.run[:12] + "-" + caseName + ".scope"
	p.rec.emit("order-step", fields)
}

func (p *orderProbe) thawCancelsAcceptedFreeze() {
	c := "thaw-cancels-accepted-freeze"
	unit, ok := p.heldScope(c)
	if !ok {
		return
	}
	freeze := backgroundBusctl(40*time.Second, "FreezeUnit", unit, 30*time.Second)
	p.emitStep(c, "freeze-started", map[string]any{})
	obs, accepted := p.waitFor(unit, 10*time.Second, observedFreezing)
	p.emitStep(c, "freeze-accepted", map[string]any{"observed": accepted, "observation": obs})
	if !accepted {
		return
	}
	p.emitStep(c, "second-freeze-returned", busctl(10*time.Second, "FreezeUnit", unit, 5*time.Second).fields())
	p.emitStep(c, "thaw-returned", busctl(10*time.Second, "ThawUnit", unit, 5*time.Second).fields())
	select {
	case r := <-freeze:
		p.emitStep(c, "freeze-returned", r.fields())
	case <-time.After(10 * time.Second):
		p.emitStep(c, "freeze-returned", map[string]any{"pending": true})
	}
	obs, _ = p.waitFor(unit, 5*time.Second, observedRunning)
	p.emitStep(c, "after-thaw", map[string]any{"observation": obs})
}

func (p *orderProbe) thawBeforeFreeze() {
	c := "thaw-before-freeze"
	unit, ok := p.heldScope(c)
	if !ok {
		return
	}
	p.emitStep(c, "thaw-returned", busctl(10*time.Second, "ThawUnit", unit, 5*time.Second).fields())
	obs, _ := p.waitFor(unit, 5*time.Second, observedRunning)
	p.emitStep(c, "after-thaw", map[string]any{"observation": obs})
	freeze := backgroundBusctl(40*time.Second, "FreezeUnit", unit, 30*time.Second)
	p.emitStep(c, "freeze-started", map[string]any{})
	obs, accepted := p.waitFor(unit, 10*time.Second, observedFreezing)
	p.emitStep(c, "late-freeze-accepted", map[string]any{"observed": accepted, "observation": obs})
	p.emitStep(c, "restore-thaw-returned", busctl(10*time.Second, "ThawUnit", unit, 5*time.Second).fields())
	select {
	case r := <-freeze:
		p.emitStep(c, "freeze-returned", r.fields())
	case <-time.After(10 * time.Second):
		p.emitStep(c, "freeze-returned", map[string]any{"pending": true})
	}
	obs, _ = p.waitFor(unit, 5*time.Second, observedRunning)
	p.emitStep(c, "after-restore", map[string]any{"observation": obs})
}

func (p *orderProbe) localTimeoutNotRollback() {
	c := "local-timeout-not-rollback"
	unit, ok := p.heldScope(c)
	if !ok {
		return
	}
	p.emitStep(c, "freeze-client-returned", busctl(10*time.Second, "FreezeUnit", unit, 2*time.Second).fields())
	obs := p.unitObservation(unit)
	p.emitStep(c, "after-client-timeout", map[string]any{"observation": obs, "still_freezing": observedFreezing(obs)})
	p.emitStep(c, "restore-thaw-returned", busctl(10*time.Second, "ThawUnit", unit, 5*time.Second).fields())
	obs, _ = p.waitFor(unit, 5*time.Second, observedRunning)
	p.emitStep(c, "after-restore", map[string]any{"observation": obs})
}

func orderMain(args []string) {
	fs := flag.NewFlagSet("order", flag.ExitOnError)
	run := fs.String("run", "", "run generation")
	state := fs.String("state", "/run/finlab/state", "fused state directory")
	fs.Parse(args)
	exe, err := os.Executable()
	if err != nil {
		fatalf(10, "executable: %v", err)
	}
	p := &orderProbe{rec: newRecorder(os.Stdout, *run, "order"), run: *run, state: *state, exe: exe}
	version, _ := commandOutput(5*time.Second, "systemd", "--version")
	p.rec.emit("order-start", map[string]any{"systemd_version": firstLine(version)})
	p.thawCancelsAcceptedFreeze()
	p.thawBeforeFreeze()
	p.localTimeoutNotRollback()
	p.rec.emit("order-end", map[string]any{})
}
