// Read-only parity oracle for the original engine. Never converts or slices.
package main
import (
  "encoding/json"
  "fmt"
  "os"
  "path/filepath"
  "github.com/nocturney/slicebridge/engine/bridge"
)
func main() {
  results := make(map[string]map[string]any)
  for _, file := range os.Args[1:] {
    result, err := bridge.InspectNozzle(file)
    if err != nil { fmt.Fprintln(os.Stderr,err); os.Exit(1) }
    results[filepath.Base(file)] = result
  }
  if err := json.NewEncoder(os.Stdout).Encode(results); err != nil {
    fmt.Fprintln(os.Stderr,err); os.Exit(1)
  }
}
