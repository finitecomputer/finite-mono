package main

import "testing"

func TestValidatePlanAcceptsSnapshotArtifact(t *testing.T) {
	plan := launchPlan{
		Name:     "finite-agent-test",
		Snapshot: "finite-runtime-20260526",
	}

	if err := validatePlan(&plan); err != nil {
		t.Fatalf("validatePlan returned error: %v", err)
	}
	if plan.MemoryMiB != 512 {
		t.Fatalf("MemoryMiB default = %d, want 512", plan.MemoryMiB)
	}
	if plan.CPUs != 1 {
		t.Fatalf("CPUs default = %d, want 1", plan.CPUs)
	}
}

func TestValidatePlanRejectsMissingArtifact(t *testing.T) {
	plan := launchPlan{Name: "finite-agent-test"}

	if err := validatePlan(&plan); err == nil {
		t.Fatal("validatePlan succeeded without image or snapshot")
	}
}

func TestValidatePlanRejectsImageAndSnapshotTogether(t *testing.T) {
	plan := launchPlan{
		Name:     "finite-agent-test",
		Image:    "python:3.11-trixie",
		Snapshot: "finite-runtime-20260526",
	}

	if err := validatePlan(&plan); err == nil {
		t.Fatal("validatePlan succeeded with both image and snapshot")
	}
}
