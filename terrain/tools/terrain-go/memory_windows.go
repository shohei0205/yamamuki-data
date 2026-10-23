package main

import (
	"golang.org/x/sys/windows"
	"path/filepath"
	"syscall"
	"unsafe"
)

func availableMemory() uint64 {
	type mem struct {
		Length, Load                                                             uint32
		Total, Avail, TotalPage, AvailPage, TotalVirtual, AvailVirtual, Extended uint64
	}
	m := mem{}
	m.Length = uint32(unsafe.Sizeof(m))
	p := syscall.NewLazyDLL("kernel32.dll").NewProc("GlobalMemoryStatusEx")
	ok, _, _ := p.Call(uintptr(unsafe.Pointer(&m)))
	if ok == 0 {
		return 1024 * 1024 * 1024
	}
	return m.Avail
}

func availableDisk(path string) (uint64, error) {
	abs, e := filepath.Abs(path)
	if e != nil {
		return 0, e
	}
	p, e := windows.UTF16PtrFromString(filepath.Dir(abs))
	if e != nil {
		return 0, e
	}
	var free, total, all uint64
	fn := syscall.NewLazyDLL("kernel32.dll").NewProc("GetDiskFreeSpaceExW")
	ok, _, err := fn.Call(uintptr(unsafe.Pointer(p)), uintptr(unsafe.Pointer(&free)), uintptr(unsafe.Pointer(&total)), uintptr(unsafe.Pointer(&all)))
	if ok == 0 {
		return 0, err
	}
	return free, nil
}
