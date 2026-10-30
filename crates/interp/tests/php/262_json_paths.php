<?php
// json_encode/json_decode across flags, escapes and errors.
class P implements JsonSerializable { function __construct(public $v) {} function jsonSerialize(): mixed { return ['wrapped' => $this->v]; } }
class Q { public $a = 1; protected $b = 2; private $c = 3; public $d = ['x' => null]; }
enum Suit: string { case H = 'hearts'; }
$strs = ["plain", "q\"uote", "back\\slash", "sl/ash", "<tag>&'amp'", "tab\there\nnl\r\x08\x0c", "\x01\x1f ctl", "é ü 漢字 😀", "", "0", "123"];
$data = ['list' => [1, 2.5, -0.0, 1e25, 0.1, true, false, null], 'strs' => $strs, 7 => 'int key', '' => 'empty key',
    'nested' => ['a' => ['b' => ['c' => []]]], 'obj' => new Q, 'ser' => new P([1, 'two']), 'enum' => Suit::H, 'empty' => new stdClass];
foreach ([0, JSON_PRETTY_PRINT, JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE, JSON_HEX_TAG | JSON_HEX_AMP | JSON_HEX_APOS | JSON_HEX_QUOT,
          JSON_FORCE_OBJECT, JSON_PRESERVE_ZERO_FRACTION, JSON_PRETTY_PRINT | JSON_FORCE_OBJECT] as $f) {
    echo json_encode($data, $f), "\n";
}
foreach ($strs as $s) echo json_encode($s), json_encode([$s => $s], JSON_UNESCAPED_UNICODE), "\n";
var_dump(json_encode("bad \xff utf8"), json_last_error(), json_last_error_msg());
var_dump(json_encode(NAN), json_last_error_msg());
$r = [1]; $r[] = &$r; var_dump(json_encode($r), json_last_error_msg());
var_dump(json_encode([[[1]]], 0, 2), json_last_error_msg());

$docs = ['{"a":1,"b":[1,2,{"c":null}],"d":"xé😀\n","e":1.5e3,"f":-0,"g":12345678901234567890}',
    '[]', '{}', '{"":1,"_empty_":2}', '{"a":1,"a":2}', '{"7":"seven","07":"oh-seven"}', '"A\/"', ' [ 1 , 2 ] ', 'true', '-12', '0.5',
    '[1,]', '{"a" 1}', '{a:1}', "\"ctl\x01\"", '"unterminated', '[1] x', '01', '1.', '"\ud800"', '[[[[1]]]]', '', 'nul'];
foreach ($docs as $d) {
    var_dump(json_decode($d), json_last_error(), json_decode($d, true), json_last_error_msg());
}
var_dump(json_decode('[[1]]', true, 1), json_last_error_msg(), json_decode('[[1]]', true, 2), json_decode('{"a":{"b":1}}', false, 3));
var_dump(json_validate('{"a":[1,2]}'), json_validate('{"a":'), json_decode('{"x":1}', null, 512, JSON_OBJECT_AS_ARRAY));
try { json_decode('{', false, 512, JSON_THROW_ON_ERROR); } catch (JsonException $e) { echo get_class($e), " ", $e->getCode(), " ", $e->getMessage(), "\n"; }
$o = json_decode('{"p":{"q":[{"r":1}]}}'); var_dump($o->p->q[0]->r, spl_object_id($o) < spl_object_id($o->p));
for ($i = 0; $i < 3; $i++) { $rt = json_decode(json_encode($data), true); echo md5(serialize($rt)), "\n"; }
