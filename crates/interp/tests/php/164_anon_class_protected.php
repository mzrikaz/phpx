<?php
function mk($i) { return new class ($i) { function __construct(protected $instance) {} function has() { return $this->instance; } }; }
function mk2($i) { return new class ($i) { function __construct(protected $instance) {} function has() { return $this->instance; } private $p = 1; function p() { return $this->p; } }; }
var_dump(mk(1)->has(), mk2(2)->has(), mk2(3)->p());
