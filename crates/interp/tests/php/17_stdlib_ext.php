<?php
class A { public $a = 1; protected $b = [1, 'x' => 2.5]; private $c = null; public $d = true; }
$s = serialize([1, 'k' => 'v', 'n' => null, 'f' => 0.1, 'o' => new A, 'neg' => -3, 'big' => 1e100]);
echo json_encode($s), "\n";
var_dump(unserialize($s));
var_dump(unserialize('a:2:{i:0;s:1:"x";i:1;b:1;}'), @unserialize('garbage'), unserialize(serialize(1.5)));
class S { public $x = 1; public function __serialize(): array { return ['y' => 2]; } public function __unserialize(array $d): void { $this->x = $d['y'] * 10; } }
echo serialize(new S), " ", unserialize(serialize(new S))->x, "\n";
echo bin2hex(pack('nvc*', 0x1234, 0x5678, 65, 66)), " ", bin2hex(pack('NA3a3H4', 1, 'ab', 'cd', 'beef')), "\n";
print_r(unpack('nfirst/vsecond/c2chars', pack('nvc*', 0x1234, 0x5678, 65, 66)));
print_r(unpack('N', "\x00\x00\x01\x00"));
echo hash('sha256', 'abc'), "\n", hash('md5', ''), "\n", sha1('abc'), "\n", hash('crc32b', 'hello'), "\n";
echo hash_hmac('sha256', 'data', 'key'), "\n", hash_hmac('sha1', 'data', str_repeat('k', 100)), "\n";
$c = hash_init('sha1'); hash_update($c, 'ab'); $c2 = hash_copy($c); hash_update($c, 'c'); echo hash_final($c), " ", hash_final($c2), "\n";
$h = hash_init('sha256', HASH_HMAC, 'key'); hash_update($h, 'data'); echo hash_final($h), "\n";
var_dump(hash_equals('abc', 'abc'), hash_equals('abc', 'abd'));
print_r(parse_url('https://user:pw@example.com:8080/p/a/t/h.php?query=1&b[]=2#frag'));
print_r(parse_url('//example.com/path')); var_dump(parse_url('/just/path?x'), parse_url('http:///bad'), parse_url('example.com:80'));
echo parse_url('http://x.com/y', PHP_URL_HOST), "\n";
parse_str('a=1&b[]=2&b[]=3&c[k][j]=v&d+e=f%20g', $out); print_r($out);
foreach ([['1.0', '1.0.0'], ['5.2', '5.10'], ['1.0rc1', '1.0'], ['1.0-dev', '1.0alpha'], ['6.4.2', '6.4.2']] as [$x, $y]) echo version_compare($x, $y), version_compare($x, $y, '>=') ? 'T' : 'F', " ";
echo "\n", strspn("42 is the answer", "1234567890"), strcspn("abcd", "cd"), strspn("foo", "o", 1, 2), " ", strpbrk('This is a test', 'st'), "\n";
echo substr_compare("Hello", "ello", 1), substr_compare("Hello", "ELLO", 1, 4, true), " ", stristr('USER@EXAMPLE.com', 'e'), " ", stristr('USER@EXAMPLE.com', '@', true), "\n";
$tok = strtok("This is\tan example\nstring", " \n\t"); while ($tok !== false) { echo "[$tok]"; $tok = strtok(" \n\t"); } echo "\n";
echo ip2long('192.168.1.1'), " ", long2ip(3232235777), " ", bin2hex(inet_pton('::1')), "\n";
