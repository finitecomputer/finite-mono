//go:build linux

package main

// A raw-protocol FUSE daemon exposing one file, "hold". A READ of it is held
// unanswered until <state>/release exists or -hold-max elapses. Receiving the
// READ means the kernel already moved the request out of its pending queue,
// which is the state of an in-flight virtiofs request in the guest (virtio_fs
// clears FR_PENDING on submission and sends no interrupts). INTERRUPT is
// recorded and ignored for the same reason.

import (
	"encoding/binary"
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"sync"
	"syscall"
	"time"
)

const (
	fuseLookup      = 1
	fuseForget      = 2
	fuseGetattr     = 3
	fuseOpen        = 14
	fuseRead        = 15
	fuseStatfs      = 17
	fuseRelease     = 18
	fuseFlush       = 25
	fuseInit        = 26
	fuseOpendir     = 27
	fuseReaddir     = 28
	fuseReleasedir  = 29
	fuseInterrupt   = 36
	fuseDestroy     = 38
	fuseBatchForget = 42

	fuseENOENT = 2
	fuseENOSYS = 38
	fuseRootID = 1
	fuseFileID = 2
)

func fuseAttr(b []byte, ino uint64) {
	le := binary.LittleEndian
	le.PutUint64(b[0:], ino)
	mode, nlink, size := uint32(0o040755), uint32(2), uint64(0)
	if ino == fuseFileID {
		mode, nlink, size = 0o100444, 1, 4096
	}
	le.PutUint64(b[8:], size)
	le.PutUint64(b[16:], (size+511)/512)
	le.PutUint32(b[60:], mode)
	le.PutUint32(b[64:], nlink)
	le.PutUint32(b[80:], 4096)
}

func fusedMain(args []string) {
	fs := flag.NewFlagSet("fused", flag.ExitOnError)
	run := fs.String("run", "", "run generation")
	mnt := fs.String("mnt", "/run/finlab/mnt", "mountpoint")
	state := fs.String("state", "/run/finlab/state", "marker directory")
	logPath := fs.String("log", "/run/finlab/log/fused.jsonl", "record file")
	holdMax := fs.Duration("hold-max", 90*time.Second, "hard upper bound on any held READ")
	fs.Parse(args)
	logFile, err := os.OpenFile(*logPath, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o644)
	if err != nil {
		fatalf(10, "open record file: %v", err)
	}
	rec := newRecorder(logFile, *run, "fused")
	fd, err := syscall.Open("/dev/fuse", syscall.O_RDWR|syscall.O_CLOEXEC, 0)
	if err != nil {
		rec.emit("fused-error", map[string]any{"step": "open", "detail": err.Error()})
		os.Exit(10)
	}
	opts := fmt.Sprintf("fd=%d,rootmode=40000,user_id=0,group_id=0,allow_other", fd)
	if err := syscall.Mount("finlab", *mnt, "fuse.finlab", syscall.MS_NOSUID|syscall.MS_NODEV, opts); err != nil {
		rec.emit("fused-error", map[string]any{"step": "mount", "detail": err.Error()})
		os.Exit(10)
	}
	rec.emit("mounted", map[string]any{"mnt": *mnt, "hold_max_s": holdMax.Seconds()})

	var wmu sync.Mutex
	reply := func(unique uint64, errno int32, payload []byte) {
		out := make([]byte, 16+len(payload))
		binary.LittleEndian.PutUint32(out[0:], uint32(len(out)))
		binary.LittleEndian.PutUint32(out[4:], uint32(-errno))
		binary.LittleEndian.PutUint64(out[8:], unique)
		copy(out[16:], payload)
		wmu.Lock()
		_, err := syscall.Write(fd, out)
		wmu.Unlock()
		if err != nil {
			rec.emit("reply-error", map[string]any{"unique": unique, "detail": err.Error()})
		}
	}
	le := binary.LittleEndian
	buf := make([]byte, 1<<20+8192)
	for {
		n, err := syscall.Read(fd, buf)
		if err == syscall.EINTR || err == syscall.ENOENT || err == syscall.EAGAIN {
			continue
		}
		if err != nil {
			rec.emit("read-exit", map[string]any{"detail": err.Error()})
			return
		}
		if n < 40 {
			continue
		}
		op, unique, node, pid := le.Uint32(buf[4:]), le.Uint64(buf[8:]), le.Uint64(buf[16:]), le.Uint32(buf[32:])
		body := append([]byte(nil), buf[40:n]...)
		switch op {
		case fuseInit:
			minor := le.Uint32(body[4:])
			out := make([]byte, 64)
			le.PutUint32(out[0:], 7)
			le.PutUint32(out[4:], min(minor, 31))
			le.PutUint32(out[8:], le.Uint32(body[8:]))
			le.PutUint32(out[20:], 128*1024)
			le.PutUint32(out[24:], 1)
			le.PutUint16(out[28:], 32)
			reply(unique, 0, out)
		case fuseLookup:
			name := string(body)
			if len(name) > 0 && name[len(name)-1] == 0 {
				name = name[:len(name)-1]
			}
			if node != fuseRootID || name != "hold" {
				reply(unique, fuseENOENT, nil)
				continue
			}
			out := make([]byte, 128)
			le.PutUint64(out[0:], fuseFileID)
			fuseAttr(out[40:], fuseFileID)
			reply(unique, 0, out)
		case fuseGetattr:
			out := make([]byte, 104)
			le.PutUint64(out[0:], 1)
			fuseAttr(out[16:], node)
			reply(unique, 0, out)
		case fuseOpen, fuseOpendir:
			out := make([]byte, 16)
			le.PutUint64(out[0:], 1)
			if op == fuseOpen {
				le.PutUint32(out[8:], 1) // FOPEN_DIRECT_IO: every read reaches the daemon
			}
			reply(unique, 0, out)
		case fuseReaddir:
			reply(unique, 0, nil)
		case fuseRead:
			size := le.Uint32(body[16:])
			rec.emit("read-received", map[string]any{"unique": unique, "pid": pid, "size": size})
			marker, _ := json.Marshal(heldRequest{Pid: int(pid), Unique: unique})
			os.WriteFile(filepath.Join(*state, "read-pending"), marker, 0o644)
			go func(unique uint64, size uint32) {
				by := "bound"
				deadline := time.Now().Add(*holdMax)
				for time.Now().Before(deadline) {
					if _, err := os.Stat(filepath.Join(*state, "release")); err == nil {
						by = "marker"
						break
					}
					time.Sleep(20 * time.Millisecond)
				}
				data := []byte("finlab-released\n")
				if int(size) < len(data) {
					data = data[:size]
				}
				rec.emit("read-released", map[string]any{"unique": unique, "by": by})
				reply(unique, 0, data)
			}(unique, size)
		case fuseInterrupt:
			rec.emit("interrupt-received", map[string]any{"unique": unique, "target_unique": le.Uint64(body[0:])})
		case fuseForget, fuseBatchForget:
		case fuseFlush, fuseRelease, fuseReleasedir, fuseDestroy:
			reply(unique, 0, nil)
		case fuseStatfs:
			reply(unique, fuseENOSYS, nil)
		default:
			reply(unique, fuseENOSYS, nil)
		}
	}
}
