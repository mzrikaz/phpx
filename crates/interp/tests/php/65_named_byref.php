<?php
class P { public array $pages = [];
  protected function disc(string $base, array &$reg, ?string $dir, ?string $ns): void { $reg[] = "$base:$dir:$ns"; }
  public function go() { $this->disc('B', $this->pages, dir: 'd', ns: 'n'); $local = []; $this->disc('L', $local, ns: 'x', dir: 'y'); return [$this->pages, $local]; } }
function addto(array &$a, string $v = 'v') { $a[] = $v; }
$p = new P; for ($i = 0; $i < 3; $i++) $r = $p->go(); var_dump($r); $z = []; addto($z, v: 'named'); var_dump($z);
