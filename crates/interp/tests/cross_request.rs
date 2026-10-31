//! Inline caches survive from one request to the next on a server worker
//! (compiled code is kept per thread), while anonymous classes and
//! rebuilt classes do not. Each test runs one script many times on one
//! thread, like a worker serving requests, switching what it does
//! through a file, and checks that nothing cached for an earlier
//! request's classes leaks into a later one.

use std::path::PathBuf;

fn mode_file(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("phpx-cross-request-{name}-{}", std::process::id()))
}

/// Run `src` once per mode on one thread; returns the outputs.
fn serve(name: &str, src: &str, modes: &str) -> Vec<String> {
    let file = mode_file(name);
    let src = src.replace("MODE_FILE", &file.to_string_lossy());
    let modes = modes.to_string();
    let out = std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || {
            modes
                .chars()
                .map(|m| {
                    std::fs::write(&file, m.to_string()).unwrap();
                    phpx_interp::run_capture(&src).0
                })
                .collect::<Vec<_>>()
        })
        .unwrap()
        .join()
        .unwrap();
    let _ = std::fs::remove_file(mode_file(name));
    out
}

const LOOPS: &str = "1500";

#[test]
fn type_check_caches_do_not_outlive_a_request() {
    let src = r#"<?php
interface Admin {}
function grant(Admin $u): string { return "granted"; }
function me(Admin $u): Admin { return $u; }
if (file_get_contents('MODE_FILE') === 'x') {
    $a = new class implements Admin {};
    for ($i = 0; $i < LOOPS; $i++) { $r = grant($a); me($a); }
    echo $r;
} else {
    $u = new class {};
    try { grant($u); echo "granted"; } catch (TypeError $e) { echo "TypeError"; }
}
"#
    .replace("LOOPS", LOOPS);
    let modes = "xyxyyxyxyy";
    for (m, out) in modes.chars().zip(serve("types", &src, modes)) {
        assert_eq!(out, if m == 'x' { "granted" } else { "TypeError" }, "mode {m}");
    }
}

#[test]
fn method_caches_do_not_outlive_a_request() {
    let src = r#"<?php
class PA { function m() { return "PA"; } }
class PB { function m() { return "PB"; } }
class PC { function m() { return "PC"; } }
function call($o) { return $o->m(); }
if (file_get_contents('MODE_FILE') === 'x') {
    $a = new class extends PA {};
    $c = new class extends PC {};
    for ($i = 0; $i < LOOPS; $i++) { $s = call($a) . call($c); }
    echo $s;
} else {
    $u = new class extends PB {};
    echo call($u);
}
"#
    .replace("LOOPS", LOOPS);
    let modes = "xyxyyxyxyy";
    for (m, out) in modes.chars().zip(serve("methods", &src, modes)) {
        assert_eq!(out, if m == 'x' { "PAPC" } else { "PB" }, "mode {m}");
    }
}

#[test]
fn rebuilt_classes_are_not_called_through_stale_caches() {
    let src = r#"<?php
function call($o) { return $o->m(); }
if (file_get_contents('MODE_FILE') === 'x') { class Base { public $bx = 1; } } else { class Base { public $by = 2; } }
class P extends Base { public $arr = [1, 2, 3]; function m() { return "P" . count($this->arr); } }
class Q extends Base { function m() { return "Q"; } }
$a = new class extends P {};
$b = new class extends Q {};
$c = new class extends P { public $z = 5; };
$out = [];
for ($i = 0; $i < LOOPS; $i++) { $out[call($a)] = 1; $out[call($b)] = 1; $out[call($c)] = 1; }
echo implode(",", array_keys($out));
"#
    .replace("LOOPS", LOOPS);
    for out in serve("rebuilt", &src, "xyxyxyxy") {
        assert_eq!(out, "P3,Q");
    }
}

#[test]
fn getter_shortcuts_do_not_outlive_a_request() {
    let src = r#"<?php
function read($o) { return $o->get(); }
if (file_get_contents('MODE_FILE') === 'x') {
    $a = new class { public $p = 'x-prop'; function get() { return $this->p; } };
    for ($i = 0; $i < LOOPS; $i++) { $r = read($a); }
    echo $r;
} else {
    $b = new class { public $q = 'y-prop'; public $p = 'wrong'; function get() { return $this->q; } };
    echo read($b);
}
"#
    .replace("LOOPS", LOOPS);
    let modes = "xyxyyxyxyy";
    for (m, out) in modes.chars().zip(serve("getters", &src, modes)) {
        assert_eq!(out, if m == 'x' { "x-prop" } else { "y-prop" }, "mode {m}");
    }
}

#[test]
fn constant_arrays_are_not_changed_by_a_request() {
    let src = r#"<?php
class Map { public static $files = ['a' => __DIR__ . '/a.php', 'n' => [1, 2], 'f' => __FILE__ . '!']; }
function lit() { return ['k' => 'v', 'list' => [1, 2, 3]]; }
$m = Map::$files; $l = lit();
$ok = $m['a'] === dirname($m['f']) . '/a.php' && $m['n'] === [1, 2] && $l['list'] === [1, 2, 3] && count($m) === 3;
Map::$files['a'] = 'changed'; Map::$files['extra'] = 1; $l['list'][] = 4;
$x = lit(); $x['k'] = 'w';
echo $ok ? 'ok' : 'BROKEN';
"#;
    for out in serve("constarrays", src, "xxxxx") {
        assert_eq!(out, "ok");
    }
}

#[test]
fn property_defaults_follow_each_request() {
    let src = r#"<?php
define('LIMIT', file_get_contents('MODE_FILE') === 'x' ? 10 : 20);
class Cfg { public $list = [1, 2]; public $limit = LIMIT; public $n = 0; }
$c = new Cfg;
$c->list[] = 3;
$c->n++;
$d = new Cfg;
echo count($d->list), " ", $d->limit, " ", $d->n;
"#;
    let modes = "xyxyyx";
    for (m, out) in modes.chars().zip(serve("defaults", src, modes)) {
        assert_eq!(out, if m == 'x' { "2 10 0" } else { "2 20 0" }, "mode {m}");
    }
}
