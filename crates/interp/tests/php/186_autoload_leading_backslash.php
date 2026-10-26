<?php spl_autoload_register(function ($c) { echo "load [$c]\n"; }); var_dump(class_exists("\\Foo\\Bar"), class_exists("Baz"), interface_exists("\\I"));
