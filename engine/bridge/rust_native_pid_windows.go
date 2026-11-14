//go:build windows

package bridge

import (
	"errors"
	"syscall"
)

// ERROR_INVALID_PARAMETER from OpenProcess means that PID does not exist.
// Every other access failure is conservatively treated as process alive.
func rustProcessAlive(pid int) bool {
	if pid <= 0 {
		return true
	}
	const queryLimitedInformation = 0x1000
	handle, err := syscall.OpenProcess(queryLimitedInformation, false, uint32(pid))
	if err != nil {
		return !errors.Is(err, syscall.Errno(87))
	}
	defer syscall.CloseHandle(handle)
	var status uint32
	if err := syscall.GetExitCodeProcess(handle, &status); err != nil {
		return true
	}
	const stillActive = 259
	return status == stillActive
}
