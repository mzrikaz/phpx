<?php
foreach (["75ply 3 layer a/net", "s/s TATTY", "o'neil mc-donald", "hello_world foo.bar", "ǆemal ǉubljana", "école ÉCOLE", "it's don't", "a1b2c3", "x\u{2019}y", "ΣΊΣΥΦΟΣ", "foo:bar", "foo·bar"] as $s) {
  echo mb_convert_case($s, MB_CASE_TITLE), " | ", mb_convert_case($s, MB_CASE_TITLE_SIMPLE), " | ", ucwords($s), " | ", ucwords($s, " /"), "\n";
}
foreach (["Straße ﬁne ŉ İstanbul ΣΑΣ σας", "ΌΣΟΣ ΣΑΣ.", "Ǆ ǅ ǆ"] as $s) { for ($m = 0; $m <= 7; $m++) echo mb_convert_case($s, $m), " | "; echo mb_strtoupper($s), " | ", mb_strtolower($s), "\n"; }
