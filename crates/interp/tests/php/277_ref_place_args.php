<?php
function setit(&$x, $v) { $x = $v; }
class C { public $p = []; function m(&$x) { $x[] = 1; } }
function g($i) { $a = []; setit($a['k'], $i); $o = new C; $n = 'p'; $n = "q"; setit($o->$n, 3); setit($o->p["z"], 4); $o->m($a['list']); return [$a, $o->p, $o->q]; }
for ($i = 0; $i < 3000; $i++) $r = g($i);
echo json_encode($r), "\n";
class Arr2 { static function accessible($v) { return is_array($v) || $v instanceof ArrayAccess; } static function exists($a, $k) { return is_array($a) ? array_key_exists($k, $a) : $a->offsetExists($k); } }
    function data_set(&$target, $key, $value, $overwrite = true)
    {
        $segments = is_array($key) ? $key : explode('.', $key);

        if (($segment = array_shift($segments)) === '*') {
            if (! Arr2::accessible($target)) {
                $target = [];
            }

            if ($segments) {
                foreach ($target as &$inner) {
                    data_set($inner, $segments, $value, $overwrite);
                }
            } elseif ($overwrite) {
                foreach ($target as &$inner) {
                    $inner = $value;
                }
            }
        } elseif (Arr2::accessible($target)) {
            if ($segments) {
                if (! Arr2::exists($target, $segment)) {
                    $target[$segment] = [];
                }

                data_set($target[$segment], $segments, $value, $overwrite);
            } elseif ($overwrite || ! Arr2::exists($target, $segment)) {
                $target[$segment] = $value;
            }
        } elseif (is_object($target)) {
            if ($segments) {
                if (! isset($target->{$segment})) {
                    $target->{$segment} = [];
                }

                data_set($target->{$segment}, $segments, $value, $overwrite);
            } elseif ($overwrite || ! isset($target->{$segment})) {
                $target->{$segment} = $value;
            }
        } else {
            $target = [];

            if ($segments) {
                data_set($target[$segment], $segments, $value, $overwrite);
            } elseif ($overwrite) {
                $target[$segment] = $value;
            }
        }

        return $target;
    }
$o = new stdClass;
for ($i = 0; $i < 200; $i++) {
    $d = ['tableFilters' => ['expense_date' => []]];
    data_set($d, 'tableFilters.expense_date.from_date', null);
    data_set($d, 'tableFilters.expense_date.to_date', $i);
    data_set($d, 'list.*', 5);
    $x = []; data_set($x, 'a.b', 1);
    data_set($o, 'p.q', $i);
    $n = null; data_set($n, 'z.y', 2);
}
echo json_encode($d), json_encode($x), json_encode($o), json_encode($n), "\n";
