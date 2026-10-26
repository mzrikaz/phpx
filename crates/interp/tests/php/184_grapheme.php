<?php
$s = "héllo wörld 👨‍👩‍👧 e\u{301}x 🇱🇰 Ǆ";
$t = function ($f, ...$a) { try { $r = $f(...$a); echo json_encode($r, JSON_UNESCAPED_UNICODE); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(); } echo "\n"; };
$t('grapheme_strlen', $s); $t('grapheme_strlen', ''); $t('grapheme_strlen', "\xff\xfe"); $t('grapheme_strlen', "abc");
foreach ([[0, null], [1, 3], [-3, null], [-3, 2], [5, -2], [100, null], [-100, 2], [12, 1], [3, 0], [0, -100]] as [$o, $l]) $t('grapheme_substr', $s, $o, $l);
foreach ([['ö', 0], ['l', 3], ['l', -3], ['O', 0], ['x', 100], ['', 0], ['👧', 0], ['e', 13], ['l', -100]] as [$n, $o]) { $t('grapheme_strpos', $s, $n, $o); $t('grapheme_stripos', $s, $n, $o); $t('grapheme_strrpos', $s, $n, $o); $t('grapheme_strripos', $s, $n, $o); }
$t('grapheme_strstr', $s, 'wö'); $t('grapheme_strstr', $s, 'wö', true); $t('grapheme_stristr', $s, 'WÖ'); $t('grapheme_stristr', $s, 'zz');
$t('grapheme_str_split', $s, 3); $t('grapheme_str_split', $s); $t('grapheme_str_split', '', 1); $t('grapheme_str_split', 'ab', 0);
foreach ([[3, GRAPHEME_EXTR_COUNT, 0], [5, GRAPHEME_EXTR_MAXBYTES, 0], [4, GRAPHEME_EXTR_MAXCHARS, 2], [2, GRAPHEME_EXTR_COUNT, 100], [2, 0, -3]] as [$sz, $ty, $of]) { $next = null; try { $r = grapheme_extract($s, $sz, $ty, $of, $next); echo json_encode($r, JSON_UNESCAPED_UNICODE), ' next=', json_encode($next), "\n"; } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; } }
if (function_exists('grapheme_levenshtein')) { $t('grapheme_levenshtein', 'kitten', 'sitting'); $t('grapheme_levenshtein', "e\u{301}", 'e'); }
