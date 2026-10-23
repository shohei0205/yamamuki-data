//go:build windows

package main

import (
	"fmt"
	"golang.org/x/sys/windows"
)

func lockOutput(path string) (func(), error) {
	p, e := windows.UTF16PtrFromString(path)
	if e != nil {
		return nil, e
	}
	h, e := windows.CreateFile(p, windows.GENERIC_READ|windows.GENERIC_WRITE, 0, nil, windows.OPEN_ALWAYS, windows.FILE_ATTRIBUTE_NORMAL, 0)
	if e != nil {
		return nil, fmt.Errorf("同じ出力が使用中です: %w", e)
	}
	return func() { windows.CloseHandle(h) }, nil
}
