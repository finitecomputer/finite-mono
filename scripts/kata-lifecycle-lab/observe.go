//go:build linux

package main

// Read-only observations of the lab container: the serving agent's identity,
// the synthetic container cgroup, task states and systemd unit state. Every
// external command runs under its own deadline.

import (
	"bufio"
	"context"
	"flag"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"sort"
	"strconv"
	"strings"
	"time"
)

func commandOutput(timeout time.Duration, name string, args ...string) (string, string) {
	ctx, cancel := context.WithTimeout(context.Background(), timeout)
	defer cancel()
	cmd := exec.CommandContext(ctx, name, args...)
	cmd.WaitDelay = 250 * time.Millisecond
	out, err := cmd.Output()
	if ctx.Err() != nil {
		return "", "timeout"
	}
	if err != nil {
		return strings.TrimSpace(string(out)), err.Error()
	}
	return strings.TrimSpace(string(out)), ""
}

func pidsByComm(comm string) []int {
	pids, _ := pidsByCommWithError(comm)
	return pids
}

func pidsByCommWithError(comm string) ([]int, string) {
	entries, err := os.ReadDir("/proc")
	if err != nil {
		return nil, err.Error()
	}
	pids := []int{}
	for _, e := range entries {
		pid, err := strconv.Atoi(e.Name())
		if err != nil {
			continue
		}
		b, err := os.ReadFile(filepath.Join("/proc", e.Name(), "comm"))
		if err != nil && !os.IsNotExist(err) {
			return nil, err.Error()
		}
		if err == nil && strings.TrimSpace(string(b)) == comm {
			pids = append(pids, pid)
		}
	}
	sort.Ints(pids)
	return pids, ""
}

func unixSocketInodes(path string) map[string]bool {
	inodes := map[string]bool{}
	f, err := os.Open("/proc/net/unix")
	if err != nil {
		return inodes
	}
	defer f.Close()
	scan := bufio.NewScanner(f)
	for scan.Scan() {
		fields := strings.Fields(scan.Text())
		if len(fields) >= 8 && fields[7] == path {
			inodes[fields[6]] = true
		}
	}
	return inodes
}

func servesSocket(pid int, path string) bool {
	inodes := unixSocketInodes(path)
	fds, _ := os.ReadDir(filepath.Join("/proc", strconv.Itoa(pid), "fd"))
	for _, fd := range fds {
		link, err := os.Readlink(filepath.Join("/proc", strconv.Itoa(pid), "fd", fd.Name()))
		if err == nil && strings.HasPrefix(link, "socket:[") && inodes[strings.TrimSuffix(strings.TrimPrefix(link, "socket:["), "]")] {
			return true
		}
	}
	return false
}

func environValue(pid int, key string) (string, bool) {
	b, err := os.ReadFile(filepath.Join("/proc", strconv.Itoa(pid), "environ"))
	if err != nil {
		return "", false
	}
	for _, kv := range strings.Split(string(b), "\x00") {
		if strings.HasPrefix(kv, key+"=") {
			return strings.TrimPrefix(kv, key+"="), true
		}
	}
	return "", false
}

func threadSummary(pid int) (workers, workersInEpoll int) {
	tasks, _ := os.ReadDir(filepath.Join("/proc", strconv.Itoa(pid), "task"))
	for _, t := range tasks {
		base := filepath.Join("/proc", strconv.Itoa(pid), "task", t.Name())
		comm, _ := os.ReadFile(base + "/comm")
		if strings.TrimSpace(string(comm)) != "tokio-rt-worker" {
			continue
		}
		workers++
		if wchan, _ := os.ReadFile(base + "/wchan"); strings.Contains(string(wchan), "epoll") {
			workersInEpoll++
		}
	}
	return workers, workersInEpoll
}

func identityMain(args []string) {
	fs := flag.NewFlagSet("identity", flag.ExitOnError)
	run := fs.String("run", "", "run generation")
	sock := fs.String("sock", "/run/fin175/agent.sock", "agent unix socket")
	fs.Parse(args)
	rec := newRecorder(os.Stdout, *run, "identity")
	fields := map[string]any{}
	fields["agent_socket"] = *sock
	fields["observation_error"] = ""
	for _, path := range []string{"/proc", "/proc/net/unix", "/proc/cmdline"} {
		if _, err := os.Stat(path); err != nil {
			fields["observation_error"] = err.Error()
		}
	}
	self, err := fileSHA256("/proc/self/exe")
	fields["helper_sha256"], fields["helper_error"] = self, errString(err)
	pids, pidError := pidsByCommWithError("kata-agent")
	if pidError != "" {
		fields["observation_error"] = pidError
	}
	fields["agent_pids"] = pids
	if len(pids) == 1 {
		pid := pids[0]
		if _, err := os.ReadFile(filepath.Join("/proc", strconv.Itoa(pid), "environ")); err != nil {
			fields["observation_error"] = err.Error()
		}
		exe, _ := os.Readlink(filepath.Join("/proc", strconv.Itoa(pid), "exe"))
		sum, err := fileSHA256(filepath.Join("/proc", strconv.Itoa(pid), "exe"))
		level, set := environValue(pid, "KATA_AGENT_LOG_LEVEL")
		addr, _ := environValue(pid, "KATA_AGENT_SERVER_ADDR")
		workers, _ := threadSummary(pid)
		fields["agent_pid"] = pid
		fields["agent_exe_path"] = exe
		fields["agent_exe_sha256"] = sum
		fields["agent_exe_error"] = errString(err)
		fields["agent_serves_socket"] = servesSocket(pid, *sock)
		fields["agent_server_addr"] = addr
		fields["agent_log_level_env_set"] = set
		fields["agent_log_level_env"] = level
		fields["agent_tokio_workers"] = workers
	}
	cmdline, cmdError := os.ReadFile("/proc/cmdline")
	if cmdError != nil {
		fields["observation_error"] = cmdError.Error()
	}
	agentParams := []string{}
	for _, word := range strings.Fields(string(cmdline)) {
		if strings.HasPrefix(word, "agent.") {
			agentParams = append(agentParams, word)
		}
	}
	fields["kernel_agent_params"] = agentParams
	release, _ := os.ReadFile("/proc/sys/kernel/osrelease")
	fields["kernel_release"] = strings.TrimSpace(string(release))
	fields["nproc"] = runtime.NumCPU()
	version, verr := commandOutput(5*time.Second, "systemd", "--version")
	fields["systemd_version"], fields["systemd_version_error"] = firstLine(version), verr
	unit, uerr := commandOutput(5*time.Second, "systemctl", "show", "-p", "ExecStart", "-p", "DropInPaths", "kata-agent.service")
	fields["agent_unit"], fields["agent_unit_error"] = unit, uerr
	_, cgerr := os.Stat("/sys/fs/cgroup/cgroup.controllers")
	fields["cgroup2"] = cgerr == nil
	rec.emit("identity", fields)
}

func errString(err error) string {
	if err == nil {
		return ""
	}
	return err.Error()
}

func firstLine(s string) string {
	if i := strings.IndexByte(s, '\n'); i >= 0 {
		return s[:i]
	}
	return s
}

func cgroupDir(mode, cid string) string {
	if mode == "systemd" {
		return "/sys/fs/cgroup/system.slice/nerdctl-" + cid + ".scope"
	}
	return "/sys/fs/cgroup/finlab/" + cid
}

func cgroupsPath(mode, cid string) string {
	if mode == "systemd" {
		return "system.slice:nerdctl:" + cid
	}
	return "/finlab/" + cid
}

func unitState(unit string) map[string]any {
	out, err := commandOutput(2*time.Second, "systemctl", "show", "-p", "ActiveState", "-p", "FreezerState", unit)
	state := map[string]any{}
	if err != "" {
		state["error"] = err
		return state
	}
	for _, line := range strings.Split(out, "\n") {
		if k, v, ok := strings.Cut(line, "="); ok {
			state[k] = v
		}
	}
	return state
}

func parsePids(list string) []int {
	pids := []int{}
	if list == "" {
		return pids
	}
	for _, p := range strings.Split(list, ",") {
		if pid, err := strconv.Atoi(strings.TrimSpace(p)); err == nil && pid > 0 {
			pids = append(pids, pid)
		} else {
			fatalf(2, "invalid retained PID %q", p)
		}
	}
	return pids
}

func observeMain(args []string) {
	fs := flag.NewFlagSet("observe", flag.ExitOnError)
	run := fs.String("run", "", "run generation")
	cid := fs.String("cid", "", "synthetic container id")
	mode := fs.String("cgroups", "systemd", "systemd|fs")
	sock := fs.String("sock", "/run/fin175/agent.sock", "agent unix socket")
	pidList := fs.String("pids", "", "comma-separated task pids seen during the arm")
	agentPid := fs.Int("agent-pid", 0, "agent pid from identity")
	label := fs.String("label", "final", "observation label")
	fs.Parse(args)
	if !cidPattern.MatchString(*cid) {
		fatalf(2, "invalid -cid")
	}
	rec := newRecorder(os.Stdout, *run, "observe")
	rec.bound = map[string]any{"cid": *cid, "operation": *run + ":stop:" + *cid}
	conn, err := dialTTRPC("observe", *sock, 3*time.Second, nil)
	fields := snapshot(context.Background(), *cid, *mode, *agentPid, parsePids(*pidList), conn)
	fields["label"] = *label
	if err != nil {
		fields["check"] = map[string]any{"outcome": "transport-error", "detail": err.Error()}
	}
	if conn != nil {
		ctx, cancel := context.WithTimeout(context.Background(), time.Second)
		defer cancel()
		if err := conn.drain(ctx); err != nil {
			fields["shutdown_error"] = err.Error()
		}
	}
	rec.emit("observation", fields)
}

// A terminal assertion applies to this full interval, never retrospectively
// to an earlier WaitProcess reply or init's pre-os.Exit marker.
func snapshot(parent context.Context, cid, mode string, agentPid int, pids []int, conn *ttrpcConn) map[string]any {
	fields := map[string]any{"started_t": time.Now().UTC().Format(time.RFC3339Nano), "cgroups_mode": mode}
	fields["cgroup_state"] = cgroupState(cgroupDir(mode, cid))
	if mode == "systemd" {
		fields["unit"] = unitState("nerdctl-" + cid + ".scope")
	}
	tasks := map[string]string{}
	for _, pid := range pids {
		tasks[strconv.Itoa(pid)] = taskState(pid)
	}
	fields["tasks"] = tasks
	agents, pidError := pidsByCommWithError("kata-agent")
	fields["observation_error"] = pidError
	fields["agent_pids"] = agents
	fields["agent_same_pid"] = len(agents) == 1 && agents[0] == agentPid
	sum, err := fileSHA256(filepath.Join("/proc", strconv.Itoa(agentPid), "exe"))
	fields["agent_exe_sha256"], fields["agent_exe_error"] = sum, errString(err)
	fields["agent_serves_socket"] = servesSocket(agentPid, "/run/fin175/agent.sock")
	if conn != nil {
		start := time.Now()
		ctx, cancel := context.WithTimeout(parent, 5*time.Second)
		r := conn.call(ctx, "grpc.Health", "Check", nil, int64(5*time.Second))
		cancel()
		check := r.fields("Check")
		check["conn"], check["latency_ms"] = conn.name, ms(time.Since(start))
		if name, detail := healthOutcome(r); name != "ok" {
			check["outcome"], check["detail"] = name, detail
		}
		if check["outcome"] == "ok" {
			check["health_status"] = 1
		}
		fields["check"] = check
	}
	fields["completed_t"] = time.Now().UTC().Format(time.RFC3339Nano)
	return fields
}

func prepareMain(args []string) {
	fs := flag.NewFlagSet("prepare", flag.ExitOnError)
	run := fs.String("run", "", "run generation")
	fs.Parse(args)
	rec := newRecorder(os.Stdout, *run, "prepare")
	root := "/var/lib/finlab/rootfs"
	for _, dir := range []string{"/run/finlab/mnt", "/run/finlab/state", "/run/finlab/log",
		root + "/proc", root + "/dev", root + "/sys", root + "/fuse", root + "/log"} {
		if err := os.MkdirAll(dir, 0o755); err != nil {
			fatalf(10, "mkdir %s: %v", dir, err)
		}
	}
	self, err := os.ReadFile("/proc/self/exe")
	if err != nil {
		fatalf(10, "read self: %v", err)
	}
	if err := os.WriteFile(root+"/finlab", self, 0o755); err != nil {
		fatalf(10, "install rootfs helper: %v", err)
	}
	sum, _ := fileSHA256(root + "/finlab")
	rec.emit("prepared", map[string]any{"rootfs": root, "rootfs_helper_sha256": sum})
}

func releaseMain(args []string) {
	fs := flag.NewFlagSet("release", flag.ExitOnError)
	run := fs.String("run", "", "run generation")
	state := fs.String("state", "/run/finlab/state", "fused state directory")
	fs.Parse(args)
	rec := newRecorder(os.Stdout, *run, "release")
	f, err := os.OpenFile(filepath.Join(*state, "release"), os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o644)
	if err != nil {
		fatalf(10, "create release marker: %v", err)
	}
	f.WriteString(*run)
	f.Close()
	rec.emit("release-requested", map[string]any{})
}
