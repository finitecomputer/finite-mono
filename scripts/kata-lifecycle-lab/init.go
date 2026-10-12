//go:build linux

package main

// Synthetic container processes. init handles SIGTERM (so the agent does not
// substitute SIGKILL, as with the Agent Runtime's bash init) and either exits
// (graceful stop) or stays. read blocks in one read(2) of a FUSE file.

import (
	"os"
	"os/exec"
	"os/signal"
	"strconv"
	"syscall"
	"time"
)

func containerRecorder(defaultPath, source string) *recorder {
	path := os.Getenv("FINLAB_LOG")
	if path == "" {
		path = defaultPath
	}
	f, err := os.OpenFile(path, os.O_WRONLY|os.O_APPEND|os.O_CREATE, 0o644)
	if err != nil {
		os.Exit(10)
	}
	return newRecorder(f, os.Getenv("FINLAB_RUN"), source)
}

func initMain() {
	rec := containerRecorder("/log/ctr.jsonl", "init")
	signals := make(chan os.Signal, 4)
	signal.Notify(signals, syscall.SIGTERM, syscall.SIGINT)
	onTerm := os.Getenv("FINLAB_INIT_ON_TERM")
	rec.emit("init-start", map[string]any{"pid_in_ns": os.Getpid(), "on_term": onTerm})
	if path := os.Getenv("FINLAB_READER"); path != "" {
		cmd := exec.Command("/finlab", "read", path)
		if err := cmd.Start(); err != nil {
			rec.emit("reader-start-error", map[string]any{"detail": err.Error()})
		} else {
			rec.emit("reader-started", map[string]any{"child_pid_in_ns": cmd.Process.Pid})
			go cmd.Wait()
		}
	}
	for s := range signals {
		rec.emit("init-signal", map[string]any{"signal": s.String()})
		if onTerm == "exit" && s == syscall.SIGTERM {
			rec.emit("init-exit", map[string]any{"status": 0})
			os.Exit(0)
		}
	}
}

func readMain(args []string) {
	if len(args) != 1 {
		fatalf(2, "usage: finlab read <path>")
	}
	// Several readers may share one record file; each process is its own source.
	rec := containerRecorder("/log/ctr.jsonl", "reader-"+strconv.Itoa(os.Getpid()))
	f, err := os.Open(args[0])
	if err != nil {
		rec.emit("reader-open-error", map[string]any{"detail": err.Error()})
		os.Exit(1)
	}
	rec.emit("reader-read-start", map[string]any{"pid_in_ns": os.Getpid()})
	b := make([]byte, 16)
	n, err := f.Read(b)
	rec.emit("reader-read-done", map[string]any{"bytes": n, "error": errString(err)})
	for {
		time.Sleep(time.Hour)
	}
}
