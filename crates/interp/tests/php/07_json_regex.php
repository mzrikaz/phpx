<?php
$data = ['name' => 'Ann "A" / é', 'age' => 30, 'score' => 9.5, 'ok' => true, 'tags' => ['a', 'b'], 'none' => null, 'empty' => [], 'obj' => new stdClass, 'f' => 1.0];
echo json_encode($data), "\n";
echo json_encode($data, JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE), "\n";
echo json_encode([1 => 'a', 2 => 'b']), json_encode([0 => 'a', 1 => 'b']), json_encode("😀"), json_encode(3.0, JSON_PRESERVE_ZERO_FRACTION), "\n";
var_dump(json_decode('{"a":1,"b":[1,2,{"c":null}],"d":1.5,"e":"é😀","big":12345678901234567890}', true));
$o = json_decode('{"x":{"y":2}}');
var_dump($o->x->y, json_decode('[1,'), json_last_error(), json_last_error_msg(), json_decode('"str"'), json_decode('nul'));
try { json_decode('{bad}', false, 512, JSON_THROW_ON_ERROR); } catch (JsonException $e) { echo "JsonException: ", $e->getMessage(), "\n"; }
class P implements JsonSerializable { public function __construct(private int $id) {} public function jsonSerialize(): mixed { return ['id' => $this->id, 'type' => 'p']; } }
echo json_encode([new P(1), new P(2)]), "\n";
// Regex
var_dump(preg_match('/(\d{4})-(\d{2})/', 'Date: 2024-05-17', $m), $m);
var_dump(preg_match('/(?<year>\d{4})-(?<mon>\d{2})/', '2024-05', $m), $m);
var_dump(preg_match_all('/\b(\w)(\w*)\b/', 'hello big world', $mm), $mm);
var_dump(preg_match_all('/(\d)/', 'a1b2', $sets, PREG_SET_ORDER), $sets);
var_dump(preg_replace('/(\w+) (\w+)/', '$2 ${1}!', 'hello world'), preg_replace('/\s+/', ' ', "a  b\t\nc"), preg_replace(['/a/', '/b/'], ['b', 'c'], 'ab'));
var_dump(preg_replace_callback('/\d+/', fn($m) => $m[0] * 2, 'a1 b22 c333'), preg_split('/[\s,]+/', "a, b  c,d"), preg_split('//', 'abc', -1, PREG_SPLIT_NO_EMPTY));
var_dump(preg_split('/(-)/', 'a-b-c', -1, PREG_SPLIT_DELIM_CAPTURE), preg_quote('Hello.World?(1+1)'), preg_match('/^abc$/i', 'ABC'), preg_match('#^/users/(\d+)$#', '/users/42', $r), $r[1]);
var_dump(preg_grep('/^\d+$/', ['1', 'a', '22']), preg_match('/colou?r/', 'The color red'), preg_replace('/(?<=@)\w+/', 'X', 'me@host'));
