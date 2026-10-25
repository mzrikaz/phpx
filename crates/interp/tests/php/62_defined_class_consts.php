<?php
enum Lv: int { case Emergency = 600; const X = 1; }
class K { const A = 5; private const P = 1; }
var_dump(defined('Lv::Emergency'), defined('Lv::Nope'), defined('K::A'), defined('K::P'), defined('Nope::A'), constant('Lv::Emergency'), constant('K::A'), defined('\K::A'));
try { constant('K::Z'); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { constant('NOPE'); } catch (Error $e) { echo $e->getMessage(), "\n"; }
