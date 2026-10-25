<?php
$a = [1, 2, 3, 'k' => 4];
echo current($a), next($a), next($a), key($a), next($a), key($a), var_export(next($a), true), " ", reset($a), end($a), prev($a), "\n";
echo html_entity_decode('&lt;p&gt; caf&eacute; &amp;amp; &#8212; &#x41; &hellip; &quot;q&quot; &#039;s&#039; &bogus;'), "\n";
echo htmlentities('<café> "q" \'s\' & © ™'), "\n";
echo json_encode(str_getcsv('a,"b,c",d""e,"f""g",,h')), "\n";
$h = fopen('php://memory', 'w+'); fputcsv($h, ['x', 'y z', 'q"t', '', 'a,b']); rewind($h); echo stream_get_contents($h); rewind($h); print_r(fgetcsv($h));
var_dump(filter_var('123', FILTER_VALIDATE_INT), filter_var('12a', FILTER_VALIDATE_INT), filter_var('yes', FILTER_VALIDATE_BOOLEAN), filter_var('a@b.co', FILTER_VALIDATE_EMAIL), filter_var('bad@', FILTER_VALIDATE_EMAIL), filter_var('http://x.com/p?q=1', FILTER_VALIDATE_URL), filter_var('nope', FILTER_VALIDATE_URL), filter_var('10.0.0.1', FILTER_VALIDATE_IP, FILTER_FLAG_NO_PRIV_RANGE), filter_var('1.5', FILTER_VALIDATE_FLOAT), filter_var('5', FILTER_VALIDATE_INT, ['options' => ['min_range' => 1, 'max_range' => 4]]));
$p = password_hash('secret', PASSWORD_DEFAULT); var_dump(strlen($p), password_verify('secret', $p), password_verify('nope', $p), substr($p, 0, 7));
var_dump(password_verify('rasmuslerdorf', '$2y$10$.vGA1O9wmRjrwAVXD98HNOgsNpDczlqm3Jq7KnEd1rVAGv3Fykk1a'));
echo crypt('rasmuslerdorf', '$2y$10$.vGA1O9wmRjrwAVXD98HNOgsNpDczlqm3Jq7KnEd1rVAGv3Fykk1a'), "\n";
echo bin2hex(gzdeflate('hello hello hello')), " ", gzinflate(gzdeflate('round trip')), " ", gzuncompress(gzcompress('zz')), " ", gzdecode(gzencode('gz')), "\n";
echo levenshtein('kitten', 'sitting'), similar_text('World', 'Word', $pct), round($pct, 2), soundex('Robert'), soundex('Tymczak'), "\n";
print_r(sscanf("age: 25 name: Bob", "age: %d name: %s")); $n = sscanf("12 apples", "%d %s", $num, $what); echo "$n $num $what\n";
class P { function a() {} private function b() {} static function c() {} } class C extends P { use T; function d() {} } trait T { function t() {} }
print_r(get_class_methods('C')); print_r(class_parents(new C)); print_r(class_uses('C'));
class V { public $x = 1; protected $y = 2; public static $s = 3; } print_r(get_class_vars('V'));
echo escapeshellarg("it's"), " ", var_export(headers_sent(), true), "\n"; header('X-A: 1'); print_r(headers_list());
register_shutdown_function(function ($x) { echo "shutdown $x\n"; }, 'arg');
echo array_sum(array_udiff([1, 5, 3], [3], fn($a, $b) => $a <=> $b)), " ", PHP_VERSION, "\n";
vprintf("%s-%d\n", ['v', 7]); fprintf(STDOUT, "%05.1f\n", 3.14159);
echo "end\n";
