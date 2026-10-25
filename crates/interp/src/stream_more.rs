//! More stream functions: stream_get_line, stream_copy_to_stream, buffer
//! and chunk sizes, socket_* aliases, context parameters.

use std::collections::HashMap;

use crate::builtins::{arg, has, int, s};
use crate::interp::BuiltinFn;
use crate::streams::with_stream;
use crate::value::{PhpArray, Value};

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    t.insert("stream_set_chunk_size", |it, _, a| {
        let n = int(it, a, 1)?;
        if n <= 0 {
            return Err(it.error("ValueError", "stream_set_chunk_size(): Argument #2 ($size) must be greater than 0"));
        }
        with_stream(it, &arg(a, 0), "stream_set_chunk_size", |_| Value::Int(8192))
    });
    for name in ["stream_set_read_buffer", "stream_set_write_buffer", "set_file_buffer"] {
        t.insert(name, |it, _, a| with_stream(it, &arg(a, 0), "stream_set_write_buffer", |_| Value::Int(0)));
    }
    t.insert("stream_supports_lock", |it, _, a| {
        with_stream(it, &arg(a, 0), "stream_supports_lock", |st| Value::Bool(matches!(st.backend, crate::streams::Backend::File(_))))
    });
    // stream_get_line($h, $length, $ending): up to $length bytes, stopping
    // before $ending (which is consumed, not returned).
    t.insert("stream_get_line", |it, _, a| {
        let max = int(it, a, 1)?;
        let ending = if has(a, 2) { s(it, a, 2)?.to_vec() } else { vec![] };
        let max = if max <= 0 { 8192 } else { max as usize };
        with_stream(it, &arg(a, 0), "stream_get_line", |st| {
            let mut out: Vec<u8> = vec![];
            loop {
                if !ending.is_empty() && out.ends_with(&ending) {
                    out.truncate(out.len() - ending.len());
                    return Value::bytes(out);
                }
                if out.len() >= max {
                    return Value::bytes(out);
                }
                let b = st.read(1, it);
                if b.is_empty() {
                    break;
                }
                out.extend_from_slice(&b);
            }
            if out.is_empty() { Value::Bool(false) } else { Value::bytes(out) }
        })
    });
    t.insert("stream_copy_to_stream", |it, _, a| {
        let max = if has(a, 2) && !arg(a, 2).is_null() { int(it, a, 2)? } else { -1 };
        let off = if has(a, 3) { int(it, a, 3)? } else { 0 };
        let (from, to) = (arg(a, 0), arg(a, 1));
        if off > 0 {
            with_stream(it, &from, "stream_copy_to_stream", |st| {
                st.read(off as usize, it);
            })?;
        }
        let mut total = 0usize;
        loop {
            let want = if max >= 0 { (max as usize).saturating_sub(total).min(65536) } else { 65536 };
            if want == 0 {
                break;
            }
            let chunk = with_stream(it, &from, "stream_copy_to_stream", |st| st.read(want, it))?;
            if chunk.is_empty() {
                break;
            }
            let n = with_stream(it, &to, "stream_copy_to_stream", |st| st.write(&chunk, it))?;
            total += n;
            if n < chunk.len() {
                break;
            }
        }
        Ok(Value::Int(total as i64))
    });
    t.insert("socket_set_blocking", |it, f, a| (it.builtins["stream_set_blocking"])(it, f, a));
    t.insert("socket_set_timeout", |it, f, a| (it.builtins["stream_set_timeout"])(it, f, a));
    t.insert("socket_get_status", |it, f, a| (it.builtins["stream_get_meta_data"])(it, f, a));
    t.insert("stream_context_set_params", |_, _, _| Ok(Value::Bool(true)));
    t.insert("stream_context_get_params", |it, f, a| {
        let opts = (it.builtins["stream_context_get_options"])(it, f, a)?;
        let mut m = PhpArray::new();
        m.insert(crate::value::Key::Str(crate::value::pstr("options")), opts);
        Ok(Value::array(m))
    });
    t.insert("stream_context_set_options", |_, _, _| Ok(Value::Bool(true)));
    t.insert("stream_context_set_default", |it, f, a| (it.builtins["stream_context_create"])(it, f, a));
}
