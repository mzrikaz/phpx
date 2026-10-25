<?php
$t = [
 ['~(*UTF8)\A[\p{Ll}\p{Lu}\p{M}\p{N}_.]+\z~', 'APP_NAME'],
 ['/(*UCP)\w+/u', 'héllo'],
 ['/^\X$/u', "e\u{301}"],
 ['/a++b/', 'aaab'],
 ['/(?<y>\d{4})-(?P<m>\d\d)/', '2024-05'],
 ['/\R/', "a\r\nb"],
 ['/\h+\v/', " \t\n"],
 ['/(?i)ABC/', 'abc'],
 ['/a(?=b)/', 'ab'],
 ['/(a)(?(1)b|c)/', 'ab'],
 ['/\Gx/', 'x'],
 ['/[[:alpha:]]+/', 'abc1'],
 ['/(?|(a)|(b))/', 'b'],
 ['/\bfoo\b/', 'a foo b'],
 ['/(?>a+)b/', 'aab'],
 ['/x*+y/', 'xxy'],
 ['/\p{Han}/u', '中'],
 ['/\Qa.b\E/', 'a.b'],
 ['/(a)\g{1}/', 'aa'],
 ['/(?<n>a)\k<n>/', 'aa'],
 ['/(?#comment)a/', 'a'],
 ['/a{,3}/', 'aaa'],
 ['/\x{263A}/u', "☺"],
 ['/[^\pL]/u', 'a1'],
 ['/(*ANY)a.b/s', "a\nb"],
 ['/(?(DEFINE)(?<d>\d))(?&d)/', '5'],
 ['/\d+(?R)?/', '12'],
 ['/a\z/m', "a\n"],
 ['/a$/', "a\n"],
 ['/\N{U+263A}/u', "☺"],
];
foreach ($t as [$p, $s]) { $r = @preg_match($p, $s, $m); echo str_pad($p, 40), var_export($r, true), ' ', json_encode($m), ' ', preg_last_error(), "\n"; }
print_r(preg_split('//', 'abc')); print_r(preg_split('//', 'abc', -1, PREG_SPLIT_NO_EMPTY)); print_r(preg_split('/(,)/', 'a,b,,c', -1, PREG_SPLIT_DELIM_CAPTURE|PREG_SPLIT_OFFSET_CAPTURE));
print_r(preg_split('/\s*/', 'a b', 2)); print_r(preg_split('/x/', 'axbxc', 2));
var_dump(preg_match_all('/a*/', 'baaa', $m), $m); var_dump(preg_match_all('/(a)(b)?/', 'aab', $m, PREG_SET_ORDER), $m);
var_dump(preg_match('/(a)(b)?(c)?/', 'a', $m, PREG_OFFSET_CAPTURE), $m); var_dump(preg_match('/(a)(b)?/', 'a', $m, PREG_UNMATCHED_AS_NULL), $m);
echo preg_replace('/(\w+) (\w+)/', '$2 ${1}! \1 \\\\ $3', 'hello world'), "\n";
echo preg_replace(['/a/', '/b/'], ['b', 'c'], 'ab'), "\n"; print_r(preg_replace('/x/', 'y', ['k' => 'xx', 'z']));
echo preg_replace_callback('/\d+/', fn($m) => $m[0] * 2, 'a1 b22', -1, $cnt), " $cnt\n";
echo preg_replace_callback_array(['/a/' => fn($m) => 'A', '/b/' => fn($m) => 'B'], 'abab', 1, $cnt), " $cnt\n";
var_dump(preg_match('/\d/', "\xff1"), preg_match('/./u', "\xff"), preg_last_error(), preg_last_error_msg());
var_dump(@preg_match('/(abc/', 'x'), preg_last_error());
echo preg_quote('Hello.World?(1+1)=2 #x/y', '/'), "\n"; print_r(preg_grep('/^\d+$/', ['1', 'a', '22'], PREG_GREP_INVERT));
var_dump(preg_match('/^abc/A', 'xabc'), preg_match('/abc/A', 'abcx'), preg_match('/a.c/U', 'abc'), preg_match('/(?<!x)y/', 'zy'));
var_dump(preg_replace('/é/u', 'e', 'café'), preg_split('//u', 'añb', -1, PREG_SPLIT_NO_EMPTY), preg_match_all('/./u', 'añ'));
