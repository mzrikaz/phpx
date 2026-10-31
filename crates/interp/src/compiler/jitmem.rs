//! Code memory for the JIT: compiled functions packed one after another.
//!
//! Cranelift's providers start fresh pages each time a function is
//! finalized, so every function began at a page boundary: thousands of
//! functions (a Laravel request compiles that many) spread over thousands
//! of mostly empty pages, and every function entry fell in the same few
//! instruction-cache sets. Here code is appended to the current chunk; the
//! page being filled goes back to read-write while the next function is
//! written into it (a module belongs to one thread, which is compiling, not
//! running code from that page, meanwhile) and is executable again when
//! the module finalizes. Data goes to Cranelift's own provider.

use std::io;

use cranelift_jit::{BranchProtection, JITMemoryKind, JITMemoryProvider, SystemMemoryProvider};
use cranelift_module::ModuleResult;

const PAGE: usize = 4096;
const CHUNK: usize = 1 << 20;

pub struct PackedCode {
    /// Current chunk and how much of it is used.
    base: *mut u8,
    len: usize,
    pos: usize,
    /// Start (page-aligned) of what was written since the last finalize.
    dirty: Option<usize>,
    data: SystemMemoryProvider,
}

// SAFETY: the raw pointers are owned memory regions, used by one thread
// at a time (the module's).
unsafe impl Send for PackedCode {}

impl PackedCode {
    pub fn new() -> PackedCode {
        PackedCode { base: std::ptr::null_mut(), len: 0, pos: 0, dirty: None, data: SystemMemoryProvider::new() }
    }

    fn protect(&self, from: usize, to: usize, prot: libc::c_int) -> io::Result<()> {
        // SAFETY: [from, to) lies within the current chunk, page-aligned.
        let r = unsafe { libc::mprotect(self.base.add(from) as *mut libc::c_void, to - from, prot) };
        if r != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn code(&mut self, size: usize, align: usize) -> io::Result<*mut u8> {
        let mut at = self.pos.next_multiple_of(align.max(1));
        if self.base.is_null() || at + size > self.len {
            // A new chunk (the old one stays: its code may still run).
            self.finalize_code()?;
            let len = size.next_multiple_of(PAGE).max(CHUNK);
            // SAFETY: a fresh anonymous mapping.
            let p = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_PRIVATE | libc::MAP_ANONYMOUS, -1, 0) };
            if p == libc::MAP_FAILED {
                return Err(io::Error::last_os_error());
            }
            self.base = p as *mut u8;
            self.len = len;
            self.pos = 0;
            self.dirty = Some(0);
            at = 0;
        } else if self.dirty.is_none() {
            // Writing into the executable page we stopped in: writable again
            // until finalized (the pages after it never were executable).
            let from = at / PAGE * PAGE;
            let to = (at + size).next_multiple_of(PAGE).min(self.len).max(from + PAGE);
            self.protect(from, to, libc::PROT_READ | libc::PROT_WRITE)?;
            self.dirty = Some(from);
        }
        self.pos = at + size;
        // SAFETY: within the chunk.
        Ok(unsafe { self.base.add(at) })
    }

    fn finalize_code(&mut self) -> io::Result<()> {
        if let Some(from) = self.dirty.take() {
            // Everything up to the end of the last page written; the rest of
            // the chunk stays read-write (never executed).
            let to = self.pos.next_multiple_of(PAGE).max(from + PAGE).min(self.len);
            self.protect(from, to, libc::PROT_READ | libc::PROT_EXEC)?;
        }
        Ok(())
    }
}

impl JITMemoryProvider for PackedCode {
    fn allocate(&mut self, size: usize, align: u64, kind: JITMemoryKind) -> io::Result<*mut u8> {
        match kind {
            JITMemoryKind::Executable => self.code(size, align as usize),
            other => self.data.allocate(size, align, other),
        }
    }

    unsafe fn free_memory(&mut self) {
        // (Code is leaked like Cranelift's: function pointers stay valid.)
        unsafe { self.data.free_memory() };
    }

    fn finalize(&mut self, branch_protection: BranchProtection) -> ModuleResult<()> {
        self.finalize_code().map_err(|e| cranelift_module::ModuleError::Backend(e.into()))?;
        self.data.finalize(branch_protection)
    }
}
