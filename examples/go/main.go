// Canary Probe — Go reference capability for Ganglion.
//
// Demonstrates the TinyGo + wit-bindgen-go toolchain for Ganglion
// capability authoring. This is the Go version of
// gang-capability-canary-probe.
//
// Build:
//
//	tinygo build -target=wasip2 -o canary-probe.wasm .
//	wasm-tools component new canary-probe.wasm -o canary-probe.component.wasm
//
// Sign and deploy:
//
//	gang sign canary-probe.component.wasm --name canary-probe --version 0.1.0
//	gang deploy robot-42 canary-probe.component.wasm
//	gang run robot-42 canary-probe
package main

import (
	"encoding/json"
	"fmt"
	"os"
)

// HealthStatus represents the overall health of a robot.
type HealthStatus string

const (
	StatusHealthy     HealthStatus = "healthy"
	StatusDegraded    HealthStatus = "degraded"
	StatusUnhealthy   HealthStatus = "unhealthy"
	StatusUnreachable HealthStatus = "unreachable"
)

// HealthCheck is a single check within the probe.
type HealthCheck struct {
	Name      string   `json:"name"`
	Passed    bool     `json:"passed"`
	Detail    string   `json:"detail"`
	Value     *float64 `json:"value,omitempty"`
	Threshold *float64 `json:"threshold,omitempty"`
}

// CanaryResult is the probe result for a single robot.
type CanaryResult struct {
	Timestamp string        `json:"timestamp"`
	Status    HealthStatus  `json:"status"`
	Checks    []HealthCheck `json:"checks"`
	ElapsedMs uint64        `json:"elapsed_ms"`
	Passed    int           `json:"passed"`
	Failed    int           `json:"failed"`
	Total     int           `json:"total"`
}

// ProbeThresholds holds configurable health thresholds.
type ProbeThresholds struct {
	MemoryWarnPct  float64
	MemoryCritPct  float64
	DiskWarnPct    float64
	DiskCritPct    float64
	MinUptimeSecs  uint64
}

// DefaultThresholds returns sensible defaults.
func DefaultThresholds() ProbeThresholds {
	return ProbeThresholds{
		MemoryWarnPct:  80.0,
		MemoryCritPct:  95.0,
		DiskWarnPct:    85.0,
		DiskCritPct:    95.0,
		MinUptimeSecs:  60,
	}
}

func floatPtr(v float64) *float64 {
	return &v
}

// CheckMemory evaluates memory usage against thresholds.
func CheckMemory(totalMB, availableMB *uint64, t ProbeThresholds) HealthCheck {
	if totalMB == nil || availableMB == nil || *totalMB == 0 {
		return HealthCheck{
			Name:   "memory",
			Passed: true,
			Detail: "memory data unavailable — skipped",
		}
	}
	usedPct := float64(*totalMB-*availableMB) / float64(*totalMB) * 100.0
	passed := usedPct < t.MemoryCritPct

	var detail string
	if usedPct >= t.MemoryCritPct {
		detail = fmt.Sprintf("CRITICAL: %.1f%% memory used", usedPct)
	} else if usedPct >= t.MemoryWarnPct {
		detail = fmt.Sprintf("WARNING: %.1f%% memory used", usedPct)
	} else {
		detail = fmt.Sprintf("%.1f%% memory used", usedPct)
	}

	return HealthCheck{
		Name:      "memory",
		Passed:    passed,
		Detail:    detail,
		Value:     floatPtr(usedPct),
		Threshold: floatPtr(t.MemoryCritPct),
	}
}

// CheckDisk evaluates disk usage against thresholds.
func CheckDisk(totalMB, usedMB *uint64, t ProbeThresholds) HealthCheck {
	if totalMB == nil || usedMB == nil || *totalMB == 0 {
		return HealthCheck{
			Name:   "disk",
			Passed: true,
			Detail: "disk data unavailable — skipped",
		}
	}
	usedPct := float64(*usedMB) / float64(*totalMB) * 100.0
	passed := usedPct < t.DiskCritPct

	var detail string
	if usedPct >= t.DiskCritPct {
		detail = fmt.Sprintf("CRITICAL: %.1f%% disk used", usedPct)
	} else if usedPct >= t.DiskWarnPct {
		detail = fmt.Sprintf("WARNING: %.1f%% disk used", usedPct)
	} else {
		detail = fmt.Sprintf("%.1f%% disk used", usedPct)
	}

	return HealthCheck{
		Name:      "disk",
		Passed:    passed,
		Detail:    detail,
		Value:     floatPtr(usedPct),
		Threshold: floatPtr(t.DiskCritPct),
	}
}

// CheckUptime evaluates system uptime.
func CheckUptime(uptimeSecs *uint64, t ProbeThresholds) HealthCheck {
	if uptimeSecs == nil {
		return HealthCheck{
			Name:   "uptime",
			Passed: true,
			Detail: "uptime data unavailable — skipped",
		}
	}
	passed := *uptimeSecs >= t.MinUptimeSecs
	var detail string
	if passed {
		detail = fmt.Sprintf("uptime %ds (ok)", *uptimeSecs)
	} else {
		detail = fmt.Sprintf("uptime %ds < %ds minimum — possible recent reboot",
			*uptimeSecs, t.MinUptimeSecs)
	}
	return HealthCheck{
		Name:      "uptime",
		Passed:    passed,
		Detail:    detail,
		Value:     floatPtr(float64(*uptimeSecs)),
		Threshold: floatPtr(float64(t.MinUptimeSecs)),
	}
}

// CheckReachable checks basic liveness.
func CheckReachable(reachable bool) HealthCheck {
	detail := "robot responded to probe"
	if !reachable {
		detail = "robot did not respond"
	}
	return HealthCheck{
		Name:   "reachable",
		Passed: reachable,
		Detail: detail,
	}
}

// Evaluate produces a CanaryResult from a set of checks.
func Evaluate(checks []HealthCheck, timestamp string, elapsedMs uint64) CanaryResult {
	passed := 0
	for _, c := range checks {
		if c.Passed {
			passed++
		}
	}
	failed := len(checks) - passed

	// Determine status
	hasUnreachable := false
	hasWarning := false
	for _, c := range checks {
		if c.Name == "reachable" && !c.Passed {
			hasUnreachable = true
		}
		if c.Passed && len(c.Detail) >= 7 && c.Detail[:7] == "WARNING" {
			hasWarning = true
		}
	}

	var status HealthStatus
	if hasUnreachable {
		status = StatusUnreachable
	} else if failed > 0 {
		status = StatusUnhealthy
	} else if hasWarning {
		status = StatusDegraded
	} else {
		status = StatusHealthy
	}

	return CanaryResult{
		Timestamp: timestamp,
		Status:    status,
		Checks:    checks,
		ElapsedMs: elapsedMs,
		Passed:    passed,
		Failed:    failed,
		Total:     len(checks),
	}
}

/*
Entry point for the WASM component.

With wit-bindgen-go, the registration would look like:

	func init() {
	    ganglion.SetExportsRun(Run)
	}

	func Run(args []string) ([]byte, error) {
	    // Call host imports via generated bindings
	    info, err := ganglion.DiagnosticsCollectSystemInfo()
	    ...
	}

This standalone version demonstrates the algorithm. When building
with TinyGo + wit-bindgen-go, replace the body of Run() with
calls to the host imports and this evaluation logic.
*/

func main() {
	// Sample data — in a real WASM component, this comes from host imports
	totalMem := uint64(8192)
	availMem := uint64(6000)
	totalDisk := uint64(102400)
	usedDisk := uint64(40000)
	uptime := uint64(86400)

	t := DefaultThresholds()

	checks := []HealthCheck{
		CheckReachable(true),
		CheckMemory(&totalMem, &availMem, t),
		CheckDisk(&totalDisk, &usedDisk, t),
		CheckUptime(&uptime, t),
	}

	result := Evaluate(checks, "2026-04-24T12:00:00Z", 15)

	data, err := json.MarshalIndent(result, "", "  ")
	if err != nil {
		fmt.Fprintf(os.Stderr, "error: %v\n", err)
		os.Exit(1)
	}
	fmt.Println(string(data))
}
