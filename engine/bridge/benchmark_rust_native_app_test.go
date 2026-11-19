package bridge

import (
	"crypto/sha256"
	"fmt"
	"os"
	"path/filepath"
	"testing"
)

// BenchmarkRustNativeAppConversion compares the same public app entrypoint
// with *request-scoped* Go versus the independently planned Rust-native path.
// It measures planning, frozen source, child startup, verification and
// exclusive publishing. It is NOT the old "Rust writer with Go plan" test.
// b.ReportAllocs covers only the Go parent, NOT the child Rust worker's RSS.
// The source is immutable; all output lives under the benchmark temp dir.
func BenchmarkRustNativeAppConversion(b *testing.B) {
	binary, oem := os.Getenv("SLICEBRIDGE_RUST_PROBE"), os.Getenv("SLICEBRIDGE_INSTALLED_BUNDLE_DIR")
	if binary == "" || oem == "" {
		b.Skip("requires explicitly approved Rust binary and read-only installed OEM snapshots")
	}
	if !filepath.IsAbs(binary) {
		b.Fatal("benchmark Rust binary must be absolute")
	}
	b.Setenv("SLICEBRIDGE_RUST_ENGINE_BIN", binary)
	sources := []string{
		filepath.Join("..", "..", "tests", "fixtures", "bambu-one-color.3mf"),
		filepath.Join("..", "..", "tests", "fixtures", "bambu-five-colors.3mf"),
	}
	if dir := os.Getenv("SLICEBRIDGE_RUST_BENCH_STRESS_DIR"); dir != "" {
		for _, name := range []string{
			"synthetic-deflate-2000.3mf",
			"synthetic-deflate-20000.3mf",
			"synthetic-deflate-60000.3mf",
			"synthetic-60000.3mf",
		} {
			path := filepath.Join(dir, name)
			if stat, err := os.Stat(path); err == nil && stat.Mode().IsRegular() {
				sources = append(sources, path)
			}
		}
	}
	for _, source := range sources {
		source := source
		info, err := os.Stat(source)
		if err != nil {
			b.Fatal(err)
		}
		if !info.Mode().IsRegular() {
			b.Fatal("source must be regular 3MF")
		}
		for _, target := range []string{"u1", "qidi"} {
			target := target
			profile := filepath.Join(oem, fmt.Sprintf("scratch-%s-0.4-resolved.json", target))
			if stat, err := os.Stat(profile); err != nil || !stat.Mode().IsRegular() {
				b.Fatalf("missing verified OEM snapshot for %s: %v", target, err)
			}
			for _, engine := range []string{"go", "rust-native"} {
				engine := engine
				b.Run(fmt.Sprintf("%s/%s/%s", filepath.Base(source), target, engine), func(b *testing.B) {
					b.SetBytes(info.Size())
					b.ReportAllocs()
					outDir := b.TempDir()
					original, err := os.ReadFile(source)
					if err != nil {
						b.Fatal(err)
					}
					hashBefore := sha256.Sum256(original)
					for i := 0; i < b.N; i++ {
						path := filepath.Join(outDir, "converted.3mf")
						req := TargetRequest{Target: target, Nozzle: "0.4", ProfileFile: profile, Engine: engine}
						report, err := ConvertTarget(source, path, req)
						if err != nil {
							b.Fatalf("app conversion %s: %v", engine, err)
						}
						if ok, _ := report["ok"].(bool); !ok {
							b.Fatalf("missing success receipt: %#v", report)
						}
						if i == 0 {
							if engine == "rust-native" && report["planner"] != "rust-native" {
								b.Fatalf("Go conversion planner was used in Rust run")
							}
						}
						b.StopTimer()
						outInfo, err := os.Stat(path)
						if err != nil || outInfo.Size() == 0 {
							b.Fatal("missing 3MF output", err)
						}
						b.ReportMetric(float64(outInfo.Size()), "output_B")
						if err := os.Remove(path); err != nil {
							b.Fatal("cannot remove private benchmark output:", err)
						}
						b.StartTimer()
					}
					b.StopTimer()
					after, err := os.ReadFile(source)
					if err != nil || sha256.Sum256(after) != hashBefore {
						b.Fatalf("benchmark mutated source: %v", err)
					}
				})
			}
		}
	}
}
