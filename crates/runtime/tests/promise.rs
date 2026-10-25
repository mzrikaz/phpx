use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use phpx_runtime::{block_on, concurrency, defer, sleep, spawn, Error, Promise};

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// Fake `DB::query` / `Http::get`: sleeps then returns a value.
fn io(label: &'static str, delay: u64) -> phpx_runtime::Result<String> {
    sleep(ms(delay))?;
    Ok(label.to_string())
}

#[test]
fn plain_code_runs_sequentially() {
    let start = Instant::now();
    let r = block_on(|| {
        let user = io("user", 50)?;
        let posts = io("posts", 50)?; // starts only after user is done
        Ok(format!("{user}+{posts}"))
    });
    assert_eq!(r.unwrap(), "user+posts");
    assert!(start.elapsed() >= ms(100));
}

#[test]
fn concurrency_run_overlaps_io() {
    let start = Instant::now();
    let r = block_on(|| {
        concurrency::run(vec![
            Box::new(|| io("user", 50)),
            Box::new(|| io("posts", 50)),
        ])
    });
    assert_eq!(r.unwrap(), vec!["user", "posts"]);
    let t = start.elapsed();
    assert!(t >= ms(50) && t < ms(90), "took {t:?}");
}

#[test]
fn async_then_wait() {
    let r = block_on(|| {
        let p = spawn(|| io("posts", 30)); // in flight
        let user = io("user", 30)?; // overlaps with p
        Ok(format!("{user}+{}", p.wait()?))
    });
    assert_eq!(r.unwrap(), "user+posts");
}

#[test]
fn then_catch_finally_chain() {
    let log = Rc::new(RefCell::new(vec![]));
    let l = log.clone();
    let r = block_on(move || {
        spawn(|| io("42", 5))
            .then(|s| s.parse::<i32>().map_err(|e| Error::exception(e.to_string())))
            .then(|n| -> phpx_runtime::Result<i32> { Err(Error::exception(format!("bad {n}"))) })
            .then(|_| -> phpx_runtime::Result<i32> { unreachable!("skipped on rejection") })
            .catch(|e| {
                assert_eq!(e, Error::exception("bad 42"));
                Ok(-1)
            })
            .finally(move || l.borrow_mut().push("finally"))
            .wait()
    });
    assert_eq!(r.unwrap(), -1);
    assert_eq!(*log.borrow(), vec!["finally"]);
}

#[test]
fn panic_becomes_error() {
    let r: phpx_runtime::Result<()> = block_on(|| spawn(|| -> phpx_runtime::Result<()> { panic!("boom") }).wait());
    assert_eq!(r, Err(Error::Panic("boom".into())));
}

#[test]
fn all_fails_fast_and_cancels_rest() {
    let start = Instant::now();
    let r = block_on(|| {
        Promise::all(vec![
            spawn(|| io("slow", 2_000)),
            spawn(|| -> phpx_runtime::Result<String> {
                sleep(ms(10))?;
                Err(Error::exception("db down"))
            }),
        ])
        .wait()
    });
    assert_eq!(r, Err(Error::exception("db down")));
    // Slow task was cancelled and its timer removed, so block_on didn't wait 2s.
    assert!(start.elapsed() < ms(200), "took {:?}", start.elapsed());
}

#[test]
fn any_race_all_settled() {
    block_on(|| {
        let first_ok = Promise::any(vec![
            spawn(|| -> phpx_runtime::Result<&str> { Err(Error::exception("mirror1 down")) }),
            spawn(|| io("mirror2", 20).map(|_| "mirror2")),
            spawn(|| io("mirror3", 500).map(|_| "mirror3")),
        ])
        .wait()?;
        assert_eq!(first_ok, "mirror2");

        let all_bad = Promise::any(vec![
            spawn(|| -> phpx_runtime::Result<()> { Err(Error::exception("a")) }),
            spawn(|| -> phpx_runtime::Result<()> { Err(Error::exception("b")) }),
        ])
        .wait();
        assert!(matches!(all_bad, Err(Error::AllRejected(e)) if e.len() == 2));

        let fastest = Promise::race(vec![
            spawn(|| io("slow", 500)),
            spawn(|| -> phpx_runtime::Result<String> {
                sleep(ms(10))?;
                Err(Error::exception("fast fail"))
            }),
        ])
        .wait();
        assert_eq!(fastest, Err(Error::exception("fast fail")));

        let settled = Promise::all_settled(vec![
            spawn(|| io("ok", 5)),
            spawn(|| -> phpx_runtime::Result<String> { Err(Error::exception("nope")) }),
        ])
        .wait()?;
        assert_eq!(settled, vec![Ok("ok".to_string()), Err(Error::exception("nope"))]);
        Ok(())
    })
    .unwrap();
}

#[test]
fn timeout_rejects_and_cancels() {
    let start = Instant::now();
    let r = block_on(|| spawn(|| io("slow", 2_000)).timeout(ms(30)).wait());
    assert_eq!(r, Err(Error::Timeout));
    assert!(start.elapsed() < ms(200));

    let r = block_on(|| spawn(|| io("fast", 5)).timeout(ms(500)).wait());
    assert_eq!(r.unwrap(), "fast");
}

#[test]
fn cancel_stops_task() {
    let r = block_on(|| {
        let p = spawn(|| io("never", 1_000));
        sleep(ms(10))?;
        p.cancel();
        Ok(p.wait())
    });
    assert_eq!(r.unwrap(), Err(Error::Cancelled));
}

#[test]
fn defer_runs_after_response() {
    let log = Rc::new(RefCell::new(vec![]));
    let l = log.clone();
    let r = block_on(move || {
        let l2 = l.clone();
        defer(move || {
            sleep(ms(10)).unwrap();
            l2.borrow_mut().push("mail sent");
        });
        l.borrow_mut().push("response");
        Ok("ok")
    });
    assert_eq!(r.unwrap(), "ok");
    assert_eq!(*log.borrow(), vec!["response", "mail sent"]);
}

#[test]
fn unawaited_promise_still_completes() {
    let done = Rc::new(RefCell::new(false));
    let d = done.clone();
    block_on(move || {
        spawn(move || {
            sleep(ms(20))?;
            *d.borrow_mut() = true;
            Ok(())
        });
        Ok(())
    })
    .unwrap();
    assert!(*done.borrow());
}

#[test]
fn ten_thousand_tasks_sleep_concurrently() {
    let start = Instant::now();
    let r = block_on(|| {
        let ps = (0..10_000).map(|i| spawn(move || sleep(ms(100)).map(|_| i))).collect();
        Ok(Promise::all(ps).wait()?.len())
    });
    assert_eq!(r.unwrap(), 10_000);
    assert!(start.elapsed() < ms(1_000), "took {:?}", start.elapsed());
}
