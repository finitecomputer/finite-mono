//go:build linux

// finlab is the in-container helper for scripts/kata_lifecycle_lab.py.
//
// It runs only inside a lab container that the runner created and owns, and
// talks only to the kata-agent unix socket and systemd of that container. It
// has no vsock transport and no host or production target. Every record it
// writes carries the run generation so the verdict can reject foreign or
// stale evidence.
//
//	finlab identity  report the serving kata-agent executable and environment
//	finlab prepare   create the synthetic container rootfs
//	finlab fused     FUSE daemon that holds READs of "hold" until released
//	finlab release   create the hold release marker
//	finlab client    drive one agent stop arm over two agent connections
//	finlab observe   one-shot final state observation on a fresh connection
//	finlab order     systemd FreezeUnit/ThawUnit ordering primitive cases
//	finlab init      synthetic container init
//	finlab read      blocks in one read(2) of a FUSE file
package main

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"regexp"
	"sync"
	"time"
)

const recordSchema = "finite.kata-lifecycle-record.v2"

var (
	runPattern = regexp.MustCompile(`^[0-9a-f]{32}$`)
	cidPattern = regexp.MustCompile(`^[0-9a-f]{64}$`)
)

type recorder struct {
	mu     sync.Mutex
	w      io.Writer
	run    string
	source string
	seq    int64
	start  time.Time
	bound  map[string]any
}

func newRecorder(w io.Writer, run, source string) *recorder {
	if !runPattern.MatchString(run) {
		fmt.Fprintln(os.Stderr, "finlab: invalid -run generation")
		os.Exit(2)
	}
	return &recorder{w: w, run: run, source: source, start: time.Now()}
}

// emit writes one JSON record. Reserved keys cannot be overwritten.
func (r *recorder) emit(kind string, fields map[string]any) {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.seq++
	record := map[string]any{
		"schema": recordSchema,
		"run":    r.run,
		"source": r.source,
		"seq":    r.seq,
		"t":      time.Now().UTC().Format(time.RFC3339Nano),
		"kind":   kind,
	}
	for key, value := range fields {
		if _, reserved := record[key]; reserved {
			panic("reserved record key " + key)
		}
		record[key] = value
	}
	for key, value := range r.bound {
		record[key] = value
	}
	line, err := json.Marshal(record)
	if err != nil {
		panic(err)
	}
	r.w.Write(append(line, '\n'))
}

func fileSHA256(path string) (string, error) {
	f, err := os.Open(path)
	if err != nil {
		return "", err
	}
	defer f.Close()
	h := sha256.New()
	if _, err := io.Copy(h, f); err != nil {
		return "", err
	}
	return hex.EncodeToString(h.Sum(nil)), nil
}

func ms(d time.Duration) float64 { return float64(d.Microseconds()) / 1000 }

func fatalf(code int, format string, args ...any) {
	fmt.Fprintf(os.Stderr, "finlab: "+format+"\n", args...)
	os.Exit(code)
}

func main() {
	if len(os.Args) < 2 {
		fatalf(2, "usage: finlab <identity|prepare|fused|release|client|observe|order|init|read> ...")
	}
	args := os.Args[2:]
	switch os.Args[1] {
	case "identity":
		identityMain(args)
	case "prepare":
		prepareMain(args)
	case "fused":
		fusedMain(args)
	case "release":
		releaseMain(args)
	case "client":
		clientMain(args)
	case "observe":
		observeMain(args)
	case "order":
		orderMain(args)
	case "init":
		initMain()
	case "read":
		readMain(args)
	default:
		fatalf(2, "unknown subcommand %q", os.Args[1])
	}
}
