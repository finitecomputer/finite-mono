package main

import (
	"os"
	"path/filepath"
	"reflect"
	"testing"
)

func nodeFiles(t *testing.T, dir, population, procs string) {
	t.Helper()
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	for name, value := range map[string]string{"cgroup.freeze": "0", "cgroup.events": "frozen 0\npopulated " + population, "cgroup.procs": procs} {
		if err := os.WriteFile(filepath.Join(dir, name), []byte(value), 0o600); err != nil {
			t.Fatal(err)
		}
	}
}

func TestCgroupDescendantInventory(t *testing.T) {
	dir := t.TempDir()
	nodeFiles(t, dir, "1", "")
	nodeFiles(t, filepath.Join(dir, "child"), "1", "173\n")
	state := cgroupState(dir)
	if state["error"] != nil || !reflect.DeepEqual(state["all_procs"], []int{173}) || len(state["descendants"].(map[string]any)) != 1 {
		t.Fatalf("descendant lost: %v", state)
	}
}

func TestCgroupMissingAndObservationErrorsDiffer(t *testing.T) {
	dir := t.TempDir()
	if state := cgroupState(filepath.Join(dir, "missing")); state["cgroup"] != "absent" {
		t.Fatal(state)
	}
	loop := filepath.Join(dir, "loop")
	if err := os.Symlink(loop, loop); err != nil {
		t.Fatal(err)
	}
	if state := cgroupState(loop); state["cgroup"] != "unknown" || state["error"] == nil {
		t.Fatalf("resolution error became absence: %v", state)
	}
	nodeFiles(t, dir, "0", "not-a-pid")
	if state := cgroupState(dir); state["error"] == nil {
		t.Fatalf("invalid PID silently discarded: %v", state)
	}
}

func TestTaskMissingAndUnreadableDiffer(t *testing.T) {
	root := t.TempDir()
	if state := taskStateAt(root, 173); state != "gone" {
		t.Fatal(state)
	}
	base := filepath.Join(root, "173")
	if err := os.Mkdir(base, 0o700); err != nil {
		t.Fatal(err)
	}
	stat := filepath.Join(base, "stat")
	if err := os.Symlink(stat, stat); err != nil {
		t.Fatal(err)
	}
	if state := taskStateAt(root, 173); state == "gone" || state[:8] != "unknown:" {
		t.Fatalf("read error became gone: %s", state)
	}
}
