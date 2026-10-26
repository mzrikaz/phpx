<?php
class B { protected static $cache = []; public static $n = 0; public static $s = 'a'; private static $priv = ['x' => [1]];
  static function put($k, $v) { static::$cache[$k] = $v; static::$cache[get_called_class()][$k] ??= $v * 2; self::$n++; ++static::$n; static::$n += 3; self::$s .= $k; return static::$cache; }
  static function del($k) { unset(static::$cache[$k]); unset(self::$priv['x'][0]); return [static::$cache, self::$priv]; }
  static function sorter() { sort(self::$priv['x']); self::$priv['x'][] = 0; sort(self::$priv['x']); return self::$priv; }
  static function whole() { self::$cache = ['reset' => true]; return self::$cache; }
}
class C extends B { }
for ($i = 0; $i < 3; $i++) { var_dump(C::put("k$i", $i)); var_dump(B::$n, B::$s); var_dump(C::del("k$i")); var_dump(B::sorter()); }
var_dump(B::whole());
class D { private static $p = 1; }
function bad() { try { D::$p = 2; } catch (Error $e) { echo $e->getMessage(), "\n"; } try { D::$zz[1] = 2; } catch (Error $e) { echo $e->getMessage(), "\n"; } }
bad(); bad();
