<?php
$src = '<?php
$a = "x $b[0] $o->p {$a[\'x\']} ${v}";
$h = <<<EOT
  a $x
  EOT;
function &f(&$a, &...$b) { return (int) $a <=> 0x1F; }
#[Attr] enum Suit: string {}
$o->class; Foo\Bar::class; // done
';
foreach (token_get_all($src) as $t) {
    echo is_array($t) ? token_name($t[0]) . ' ' . $t[2] . ' ' . json_encode($t[1]) : json_encode($t), "\n";
}
class MyToken extends PhpToken {}
$toks = MyToken::tokenize('<?php echo $x; ');
echo get_class($toks[0]), ' ', count($toks), "\n";
foreach ($toks as $t) {
    echo $t->getTokenName(), ' ', $t->pos, ' ', $t->line, ' ', var_export($t->isIgnorable(), true), ' ', var_export($t->is([T_ECHO, ';']), true), "\n";
}
echo token_name(T_DOUBLE_COLON), ' ', token_name(59), "\n";
