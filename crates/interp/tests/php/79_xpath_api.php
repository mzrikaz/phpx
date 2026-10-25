<?php
$d = new DOMDocument; $d->loadXML('<r xmlns:a="urn:a"><a:x id="1">one</a:x><y n="2">two</y><y n="3">three</y><!--c--></r>');
$x = new DOMXPath($d);
var_dump($x->query('//y')->length, $x->evaluate('count(//y)'), $x->evaluate('string(//y[2]/@n)'), $x->evaluate('//y[1] = "two"'));
var_dump($x->query('count(//y)')->length);
var_dump(@$x->query('//['));
$r = $x->query('//[');
var_dump($x->query('//b:x'));
var_dump($x->query('//a:x')->length);
$x->registerNamespace('z', 'urn:a');
var_dump($x->query('//z:x')->item(0)->nodeValue, $x->evaluate('name(//z:x)'), $x->evaluate('1 div 3'), $x->evaluate('0 div 0'), $x->evaluate('-1 div 0'), $x->evaluate('string(1.0)'), $x->evaluate('string(0.1 + 0.2)'));
var_dump($x->evaluate('//y/@n')->item(1)->value, get_class($x->document), $x->query('//comment()')->length, $x->query('/')->item(0)->nodeName);
$h = new DOMDocument; @$h->loadHTML('<div class="a b"><p>x</p><P>y</P></div>');
$hx = new DOMXPath($h); var_dump($hx->query('//P')->length, $hx->query('//p')->length, $hx->query("//div[contains(concat(' ', normalize-space(@class), ' '), ' b ')]")->length);
var_dump($x->evaluate('//y[last()]/text()')->item(0)->data, $x->evaluate('sum(//y/@n)'), $x->evaluate('substring("12345", 1.5, 2.6)'), $x->evaluate('translate("bar","abc","ABC")'));
