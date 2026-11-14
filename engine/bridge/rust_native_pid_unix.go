//go:build !windows

package bridge

import (
	"errors"
	"syscall"
)

// Unknown permission failures are treated as alive: false negatives could
// erase another live process's scratch.
func rustProcessAlive(pid int) bool {
	if pid <= 0 {
		return true
	}
	err := syscall.Kill(pid, 0)
	return !errors.Is(err, syscall.ESRCH)
}
