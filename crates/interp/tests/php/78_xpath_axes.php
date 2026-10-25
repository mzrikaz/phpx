<?php
$d = new DOMDocument;
$d->loadXML('<lib><shelf id="s1"><book year="2001" lang="en"><title>A</title><price>10.5</price></book><book year="1999"><title>B</title><price>20</price></book></shelf><shelf id="s2"><book year="2010" xml:lang="en-GB"><title>C</title><price>5</price></book><!-- note --><?pi data?></shelf></lib>');
$x = new DOMXPath($d);
$show = function ($e, $ctx = null) use ($x) {
    $r = $ctx ? $x->evaluate($e, $ctx) : $x->evaluate($e);
    if ($r instanceof DOMNodeList) {
        $o = [];
        foreach ($r as $n) $o[] = $n->nodeType === 1 ? $n->nodeName . ($n->hasAttribute('year') ? '@' . $n->getAttribute('year') : '') : $n->nodeName . '=' . trim((string) $n->nodeValue);
        echo str_pad($e, 45), implode(' ', $o), "\n";
    } else {
        echo str_pad($e, 45); var_dump($r);
    }
};
foreach ([
    '/lib/shelf/book', '//book[@year > 2000]', '//book[last()]', '//book[1]', '(//book)[1]', '//book[position() = 2]',
    '//title/text()', '//book/@year', '//shelf[@id="s2"]/book/title', '//book[price < 10]/title', 'count(//book)',
    'sum(//price)', '//book/ancestor::shelf', '//title[.="C"]/ancestor::*', '//book[2]/preceding-sibling::book',
    '//title[.="A"]/following::title', '//title[.="C"]/preceding::title', '//book[title="B"]/following-sibling::*',
    '//shelf[1]/descendant::*', '//shelf[2]/node()', '//comment()', '//processing-instruction()', '//processing-instruction("pi")',
    '//book[lang("en")]', '//title | //price', '//price | //title', 'string(//book[2]/@year)', 'number("12.50")', 'number("abc")',
    'concat("a", "b", 3)', 'normalize-space("  a  b ")', 'string-length("héllo")', 'substring("hello", 2)', 'substring-before("a-b", "-")',
    'starts-with("phpx", "ph")', 'boolean(//nothing)', 'not(//book)', 'floor(2.7)', 'ceiling(2.1)', 'round(2.5)', 'round(-2.5)',
    '10 mod 3', '-10 mod 3', '7 div 2', '1 = 1.0', '"1" = 1', '//book/@year = 1999', '//book/@year != 1999', 'true() and false()',
    'local-name(//book[1]/*)', 'name(/*)', '//*[local-name()="price"][. > 6]', '//book[title][price > 1][2]', '//shelf/book[1]',
    '//*[@*]', '//@*', 'id("s2")', '/', '/*', '//book[../@id="s1"]', '//book[not(@lang)]', '.', '..',
] as $e) $show($e);
$show('title', $x->query('//book')->item(2));
$show('../../@id', $x->query('//title')->item(0));
$show('./following::book', $x->query('//book')->item(0));
