<?php
namespace App;
// Constant reads from the source are cached per name; a namespaced name
// that fell back to the global constant sees a later namespaced define().
const LOCAL = 'local';
function read() { return [\PHP_EOL === "\n", LOCAL, FOO, \App\LOCAL, namespace\LOCAL]; }
define('FOO', 'global foo');
for ($i = 0; $i < 3; $i++) {
    echo json_encode(read()), "\n";
    if ($i == 1) define('App\FOO', 'namespaced foo');
}
function dep() { return \DATE_RFC7231; }
set_error_handler(function ($no, $msg) { echo "E$no: $msg\n"; return true; });
for ($i = 0; $i < 2; $i++) echo dep(), "\n";
function undefined() { try { return NOPE; } catch (\Error $e) { return $e->getMessage(); } }
echo undefined(), "\n"; define('App\NOPE', 1); echo undefined(), "\n";
