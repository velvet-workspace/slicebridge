package main

import (
	"context"
	"fmt"
	"os"
)

// An empty request engine preserves the existing developer env selector and
// installed Go default. Explicit per-call selection never changes process
// environment, so concurrent printer jobs cannot change one another's mode.
func effectiveConversionEngine(requested string) (string, error) {
	engine := requested
	if engine == "" {
		engine = os.Getenv("SLICEBRIDGE_CONVERSION_ENGINE")
	}
	switch engine {
	case "", "go", "rust", "rust-native":
		return engine, nil
	default:
		return "", fmt.Errorf("unknown experimental conversion engine")
	}
}

// Rust-native jobs are cancellable as a group from the Wails application.
// Go conversions retain their existing behavior and are not advertised as
// cancellable in-flight.
func (a *App) beginConversion(engine string) (context.Context, func()) {
	parent := a.ctx
	if parent == nil {
		parent = context.Background()
	}
	if engine != "rust-native" {
		return parent, func() {}
	}
	ctx, cancel := context.WithCancel(parent)
	a.mutex.Lock()
	a.conversionSeq++
	id := a.conversionSeq
	if a.conversionCancels == nil {
		a.conversionCancels = make(map[uint64]context.CancelFunc)
	}
	a.conversionCancels[id] = cancel
	a.mutex.Unlock()
	return ctx, func() {
		a.mutex.Lock()
		delete(a.conversionCancels, id)
		a.mutex.Unlock()
		cancel()
	}
}

// CancelConversion cancels all currently active Rust-native conversion
// requests without touching the user's slicer or Go jobs. The native worker
// is killed by exec.CommandContext, and its scratch directory is cleaned.
func (a *App) CancelConversion() bool {
	a.mutex.Lock()
	cancels := make([]context.CancelFunc, 0, len(a.conversionCancels))
	for _, cancel := range a.conversionCancels {
		cancels = append(cancels, cancel)
	}
	a.mutex.Unlock()
	for _, cancel := range cancels {
		cancel()
	}
	return len(cancels) > 0
}
