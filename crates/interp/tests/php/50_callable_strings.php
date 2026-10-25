#!/usr/bin/env php
<?php
namespace Lib;
final class Loader { public static function load(string $c): void { echo "load $c\n"; } }
spl_autoload_register([Loader::class, 'load']);
var_dump(class_exists('Missing\Thing'));
echo call_user_func('Lib\Loader::load', 'x'), call_user_func(['\Lib\Loader', 'load'], 'y');
var_dump(is_callable(['Lib\Loader', 'load']), __LINE__);
