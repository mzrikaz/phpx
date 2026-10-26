<?php
ini_set('session.serialize_handler', 'php_binary');
ini_set('session.save_path', sys_get_temp_dir());
session_id('pxbintest212');
@session_start();
$_SESSION['user'] = ['id' => 7, 'name' => "Ri\0kaz"]; $_SESSION['n'] = 1.5; $_SESSION[str_repeat('k', 130)] = 'skipped';
$enc = session_encode();
echo bin2hex($enc), "\n";
$_SESSION = [];
var_dump(session_decode($enc), $_SESSION);
var_dump(session_decode(chr(128 | 3) . 'abc' . chr(1) . 'x' . 'i:5;'), $_SESSION);
session_destroy();
