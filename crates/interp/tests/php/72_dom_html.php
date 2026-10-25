<?php

$cases = [
 '<?xml encoding="UTF-8"><!DOCTYPE html><html><body><div class="a b">héllo <b>x</b> &amp; &nbsp;y</div><br><p>one<p>two</body></html>',
 'hello',
 '<p>a<div>b</div>',
 '<ul><li>a<li>b</ul><table><tr><td>1<td>2</table>',
 '<!-- c --><span title=\'q"\' data-x=1 checked>t</span><script>if (a<b) x="</p>";</script>',
 '<head><title>T</title></head><body>B</body>',
 '<div><unknown-tag a=b>u</unknown-tag></div>',
 "<div>\n  <span>x</span>\n</div>",
];
foreach ($cases as $h) {
  $d = new DOMDocument; $d->loadHTML($h, LIBXML_NOERROR);
  echo "---\n", $d->saveHTML();
  $d = new DOMDocument; $d->loadHTML($h, LIBXML_NOERROR | LIBXML_HTML_NODEFDTD | LIBXML_NOBLANKS| LIBXML_HTML_NOIMPLIED);
  echo "~~\n", $d->saveHTML(), "~~xml\n", $d->saveXML();
}
$x = new DOMDocument; $x->loadXML("<r a=\"é\n\"><b>é\r</b><c/><!--c--><?p d?></r>");
echo $x->saveXML(), $x->saveXML($x->documentElement), "\n";
$x = new DOMDocument('1.0', 'UTF-8'); $x->loadXML("<?xml version=\"1.0\" encoding=\"UTF-8\"?><r a=\"é\"><b>é</b></r>");
echo $x->saveXML(), $x->saveXML($x->documentElement), "\n";
$x->formatOutput = true; $r=$x->documentElement; $r->appendChild($x->createElement('k'))->appendChild($x->createElement('z','t'));
echo $x->saveXML();
$y = new DOMDocument('1.0', 'UTF-8'); $e=$y->createElement('a'); $y->appendChild($e); $e->appendChild($y->createElement('b', 'é')); echo $y->saveXML(), $y->saveXML($e), "\n";
$h = new DOMDocument; $h->loadHTML('<p>é</p><script>a</script>'); $s=$h->getElementsByTagName('script')->item(0); var_dump($s->firstChild->nodeType); echo $h->saveXML($h->getElementsByTagName('p')->item(0)),"\n";
var_dump($h->encoding);
$h = new DOMDocument; $h->loadHTML('<meta charset="utf-8"><p>é</p>'); var_dump($h->encoding);
$h->formatOutput = true; echo $h->saveHTML();
$h = new DOMDocument; $h->loadHTML('<div><p>a</p><ul><li>x</li><li>y</li></ul></div>'); $h->formatOutput = true; echo $h->saveHTML();
foreach (['<p>e</p>', '<p>é</p>'] as $src) {
$h = new DOMDocument; $h->loadHTML($src); var_dump($h->encoding); echo $h->saveXML(), $h->saveHTML();
}
$h = new DOMDocument; $h->loadHTML('<p title="é">é</p>'); echo $h->saveHTML($h->getElementsByTagName('p')->item(0)), "\n";
