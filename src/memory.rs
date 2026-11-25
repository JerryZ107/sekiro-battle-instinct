//! Small, allocation-free memory validation helpers.
//!
//! The pattern is deliberately borrowed from the two upstream projects:
//! dec32/sekiro-battle-instinct keeps all game pointer chains in one place,
//! while kargoshakar/SekiroImGuiCustom validates a pointer with a cheap
//! `ReadProcessMemory` probe instead of issuing `VirtualQuery` from frame code.
//!
//! `is_valid_read_ptr` probes the first and last byte only. That is enough for
//! the contiguous game allocations used by the inventory scanner; callers that
//! need stronger guarantees should validate the range once when the scan starts
//! and then cache the result rather than re-probing every frame.

use std::ffi::c_void;

use windows::Win32::{
    Foundation::HANDLE,
    System::{Diagnostics::Debug::ReadProcessMemory, Threading::GetCurrentProcess},
};

#[inline]
pub(crate) fn is_valid_read_ptr(addr: usize, len: usize) -> bool {
    if addr == 0 || len == 0 {
        return false;
    }
    let Some(last) = addr.checked_add(len - 1) else {
        return false;
    };

    unsafe {
        let process: HANDLE = GetCurrentProcess();
        let mut byte = 0u8;
        let mut read = 0usize;
        for probe in [addr, last] {
            let ok = ReadProcessMemory(
                process,
                probe as *const c_void,
                &mut byte as *mut u8 as *mut c_void,
                1,
                Some(&mut read),
            )
            .is_ok();
            if !ok || read != 1 {
                return false;
            }
        }
    }
    true
}

/// Copy a whole region out of the current process with one syscall. The
/// inventory scanner parses this private buffer instead of dereferencing game
/// memory row by row.
pub(crate) fn read_bytes(addr: usize, out: &mut [u8]) -> bool {
    if addr == 0 || out.is_empty() {
        return false;
    }
    unsafe {
        let process: HANDLE = GetCurrentProcess();
        let mut read = 0usize;
        let ok = ReadProcessMemory(
            process,
            addr as *const c_void,
            out.as_mut_ptr() as *mut c_void,
            out.len(),
            Some(&mut read),
        )
        .is_ok();
        ok && read == out.len()
    }
}

#[inline]
pub(crate) unsafe fn read_u16(base: *const u8, offset: usize) -> u16 {
    unsafe { u16::from_le((base.add(offset) as *const u16).read_unaligned()) }
}

#[inline]
pub(crate) unsafe fn read_u32(base: *const u8, offset: usize) -> u32 {
    unsafe { u32::from_le((base.add(offset) as *const u32).read_unaligned()) }
}

#[inline]
pub(crate) unsafe fn write_u16(base: *mut u8, offset: usize, value: u16) {
    unsafe { (base.add(offset) as *mut u16).write_unaligned(value.to_le()) };
}

#[inline]
pub(crate) unsafe fn read_i32(base: *const u8, offset: usize) -> i32 {
    unsafe { i32::from_le((base.add(offset) as *const i32).read_unaligned()) }
}

#[inline]
pub(crate) unsafe fn write_i32(base: *mut u8, offset: usize, value: i32) {
    unsafe { (base.add(offset) as *mut i32).write_unaligned(value.to_le()) };
}
