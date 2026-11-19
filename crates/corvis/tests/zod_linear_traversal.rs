//! Recursive schemas traverse their input in linear time: each level moves its children instead of copying its whole
//! subtree. Measured as bytes allocated by a parse, which a quadratic traversal doubles twice when the depth doubles.
//! This is the only test of the binary because the allocator counts the whole process.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use corvis::z::{self, ParseOptions, ZodSchema};
use serde_json::{Value, json};

struct Counting;

/// Bytes allocated since the start (a reallocation counts its new size).
static ALLOCATED: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATED.fetch_add(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATED.fetch_add(new_size, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn node() -> z::Schema {
    z::Schema::from(z::object! { c: z::lazy(node).nullable(), pad: z::array(z::string()) })
}

fn obj(entries: Vec<(&str, Value)>) -> Value {
    Value::Object(entries.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// `{ c: { c: ... { c: null, pad }, pad }, pad }`, built by moving each level into the next.
fn nested(depth: usize) -> Value {
    let pad = json!(["x".repeat(64), "y".repeat(64), "z".repeat(64), "w".repeat(64)]);
    let mut v = obj(vec![("c", Value::Null), ("pad", pad.clone())]);
    for _ in 0..depth {
        v = obj(vec![("c", v), ("pad", pad.clone())]);
    }
    v
}

/// Bytes allocated by `parse` of `nested(depth)`; the input and output are leaked, since a value this deep cannot be
/// dropped recursively on every stack and dropping allocates nothing anyway.
fn allocated_by(depth: usize, parse: &dyn Fn(&Value, &ParseOptions) -> Value) -> usize {
    let v = nested(depth);
    let opts = ParseOptions::new().max_depth(depth + 10);
    let before = ALLOCATED.load(Ordering::Relaxed);
    let out = parse(&v, &opts);
    let bytes = ALLOCATED.load(Ordering::Relaxed) - before;
    assert_eq!(out, v);
    std::mem::forget((v, out));
    bytes
}

#[test]
fn recursive_traversal_is_linear() {
    std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024)
        .spawn(|| {
            let schemas: [(&str, &dyn Fn(&Value, &ParseOptions) -> Value); 2] = [
                ("z.lazy object", &|v, opts| node().parse_with(v, opts).unwrap()),
                ("z.json()", &|v, opts| z::json().parse_with(v, opts).unwrap()),
            ];
            for (name, parse) in schemas {
                let n = allocated_by(1500, parse);
                let two_n = allocated_by(3000, parse);
                // Linear: about 2. Copying each subtree at every level (quadratic): about 4.
                let ratio = two_n as f64 / n as f64;
                assert!(ratio <= 2.2, "{name}: depth 3000 allocated {two_n} bytes, depth 1500 {n} (ratio {ratio:.2})");
            }
        })
        .unwrap()
        .join()
        .unwrap();
}
