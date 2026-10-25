//! What the compiler will emit for this phpx controller:
//!
//! ```php
//! function show(int $id): Response {
//!     $user = DB::find($id);                                      // sequential
//!     [$posts, $friends] = Concurrency::run([                     // parallel
//!         fn() => Http::get("/posts/{$user->ext_id}"),
//!         fn() => DB::friendsOf($user->id),
//!     ]);
//!     defer(fn() => Analytics::track('profile', $id));            // after response
//!     return view('profile', compact('user', 'posts', 'friends'));
//! }
//! ```
//!
//! Run: `cargo run --example profile`

use std::time::{Duration, Instant};

use phpx_runtime::{block_on, concurrency, defer, sleep, Result};

fn fake_io(what: &str, ms: u64) -> Result<String> {
    sleep(Duration::from_millis(ms))?;
    Ok(what.to_string())
}

fn show(id: u32) -> Result<String> {
    let t = Instant::now();
    let user = fake_io(&format!("user#{id}"), 40)?;
    println!("  [{:>3}ms] DB::find done        -> {user}", t.elapsed().as_millis());

    let user2 = user.clone();
    let r = concurrency::run(vec![
        Box::new(move || fake_io(&format!("posts of {user2}"), 80)),
        Box::new(|| fake_io("friends", 60)),
    ])?;
    println!("  [{:>3}ms] Concurrency::run done -> {:?}", t.elapsed().as_millis(), r);

    defer(move || {
        let _ = sleep(Duration::from_millis(50));
        println!("  [{:>3}ms] deferred analytics sent", t.elapsed().as_millis());
    });
    println!("  [{:>3}ms] response returned", t.elapsed().as_millis());
    Ok(format!("profile({user})"))
}

fn main() {
    println!("one request (expect ~120ms: 40 sequential + max(80, 60) parallel):");
    let res = block_on(|| show(7));
    println!("  => {res:?}\n");

    println!("1000 requests at once on one OS thread:");
    let t = Instant::now();
    let n = block_on(|| {
        let reqs = (0..1000).map(|i| phpx_runtime::spawn(move || fake_io(&i.to_string(), 100))).collect();
        Ok(phpx_runtime::Promise::all(reqs).wait()?.len())
    });
    println!("  => {n:?} done in {}ms (sequential would be 100000ms)", t.elapsed().as_millis());
}
