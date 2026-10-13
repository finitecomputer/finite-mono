package main

// Synthetic cgroup/task observation. Errors stay unknown; descendants are retained.
import (
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
)

func readTrim(path string) (string, bool) {
	b, err := os.ReadFile(path)
	if err != nil {
		return "", false
	}
	return strings.TrimSpace(string(b)), true
}

// cgroupState returns the synthetic container cgroup files; "absent" is a value, not an error.
func cgroupState(dir string) map[string]any {
	state := map[string]any{"path": dir}
	if _, err := os.Stat(dir); err != nil {
		state["cgroup"] = "unknown"
		if os.IsNotExist(err) {
			state["cgroup"] = "absent"
		} else {
			state["error"] = err.Error()
		}
		return state
	}
	state["cgroup"] = "present"
	seen := map[int]bool{}
	descendants := map[string]any{}
	readNode := func(path string, node map[string]any) error {
		for _, file := range []string{"cgroup.freeze", "cgroup.events", "cgroup.procs"} {
			b, err := os.ReadFile(filepath.Join(path, file))
			if err != nil {
				return err
			}
			switch file {
			case "cgroup.freeze":
				node["freeze"] = strings.TrimSpace(string(b))
			case "cgroup.events":
				for _, line := range strings.Split(strings.TrimSpace(string(b)), "\n") {
					parts := strings.Fields(line)
					if len(parts) != 2 {
						return fmt.Errorf("malformed cgroup.events: %s", path)
					}
					node[parts[0]] = parts[1]
				}
			case "cgroup.procs":
				procs := []int{}
				for _, token := range strings.Fields(string(b)) {
					pid, err := strconv.Atoi(token)
					if err != nil || pid <= 0 {
						return fmt.Errorf("invalid cgroup PID %q", token)
					}
					procs = append(procs, pid)
					seen[pid] = true
				}
				node["procs"] = procs
			}
		}
		return nil
	}
	err := filepath.WalkDir(dir, func(path string, entry os.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if !entry.IsDir() {
			return nil
		}
		node := state
		if path != dir {
			node = map[string]any{}
			rel, _ := filepath.Rel(dir, path)
			descendants[rel] = node
		}
		return readNode(path, node)
	})
	if err != nil {
		state["error"] = err.Error()
	}
	all := []int{}
	for pid := range seen {
		all = append(all, pid)
	}
	sort.Ints(all)
	state["descendants"], state["all_procs"] = descendants, all
	return state
}

// taskState is "<state>:<wchan>" from /proc, or "gone".
func taskState(pid int) string { return taskStateAt("/proc", pid) }

func taskStateAt(procRoot string, pid int) string {
	stat, err := os.ReadFile(filepath.Join(procRoot, strconv.Itoa(pid), "stat"))
	if err != nil {
		if os.IsNotExist(err) {
			return "gone"
		}
		return "unknown:" + err.Error()
	}
	s := string(stat)
	i := strings.LastIndexByte(s, ')')
	if i < 0 || i+2 >= len(s) {
		return "unknown"
	}
	wchan, ok := readTrim(filepath.Join(procRoot, strconv.Itoa(pid), "wchan"))
	if !ok {
		return "unknown:wchan unreadable"
	}
	return string(s[i+2]) + ":" + wchan
}
