<?php
$vals = [0, 1, -1, 1234.5678, -1234.5678, 0.125, 0.135, 2.5, 3.5, 1.005, 99.9995, 1e-7, -0.0005, 1e20, 1234567.891, 0.256, 123456789012345678, PHP_INT_MAX, -0.0, NAN, INF, -INF, 0.1 + 0.2, 1.5e300, 7, 1000, 12345];
foreach (["en", "en_US", "en_IN", "de", "fr", "fr_CH", "hi", "ar", "bn", "si", "ta", "ja", "es", "pt_BR", "ru", "sv", "nb", "de_CH", "it", "pl", "nl", "zh", "ko", "fa", "he", "tr", "en_GB", "en_AU", "sr_Latn_RS", "en-US", "EN", "en_XX"] as $l) {
  foreach ([1, 2, 3, 12, 4] as $st) {
    $f = new NumberFormatter($l, $st);
    $out = [];
    foreach ($vals as $v) $out[] = $f->format($v);
    echo "$l/$st [", $f->getPattern(), "] ", $f->getLocale(), ": ", implode(" | ", $out), "\n";
  }
  $c = new NumberFormatter($l, 2);
  echo "  cur: ", implode(" | ", array_map(fn($x) => $c->formatCurrency(-1234.567, $x), ["USD", "EUR", "GBP", "JPY", "INR", "LKR", "BHD", "CHF", "XYZ", "AUD", "CNY", "KRW", "RUB", "BRL"])), "\n";
}
$f = new NumberFormatter("en", 1);
$f->setAttribute(NumberFormatter::FRACTION_DIGITS, 2); echo $f->format(1.005), " ", $f->format(1), " ", $f->getPattern(), "\n";
$f->setAttribute(NumberFormatter::ROUNDING_MODE, NumberFormatter::ROUND_HALFUP); echo $f->format(0.125), " ", $f->format(0.135), " ", $f->format(1.005), "\n";
foreach ([0,1,2,3,5,6,8] as $m) { $f->setAttribute(NumberFormatter::ROUNDING_MODE, $m); echo $m, ":", $f->format(2.345), ",", $f->format(-2.345), ",", $f->format(2.355), ",", $f->format(-2.355), " "; } echo "\n";
$f->setAttribute(NumberFormatter::MAX_FRACTION_DIGITS, 1); echo $f->format(1.25), " ", $f->getPattern(), "\n";
$f->setAttribute(NumberFormatter::GROUPING_USED, 0); echo $f->format(12345.6), "\n";
$g = new NumberFormatter("en", NumberFormatter::PATTERN_DECIMAL, "#,##0.00;(#,##0.00)"); echo $g->format(-1234.5), " ", $g->getPattern(), "\n";
$g = new NumberFormatter("en", NumberFormatter::PATTERN_DECIMAL, "00000.###"); echo $g->format(12.3456), "\n";
$g = new NumberFormatter("en", NumberFormatter::PATTERN_DECIMAL, "@@@"); echo $g->format(12345), " ", $g->format(0.012345), " ", $g->format(1.5), "\n";
$g = new NumberFormatter("en", NumberFormatter::PATTERN_DECIMAL, "'#'#,##0 'units'"); echo $g->format(1234), "\n";
$c = new NumberFormatter("en_US", 2); $c->setAttribute(NumberFormatter::FRACTION_DIGITS, 0); echo $c->formatCurrency(1234.567, "USD"), " ", $c->format(5), "\n";
$p = new NumberFormatter("en", 3); $p->setAttribute(NumberFormatter::MAX_FRACTION_DIGITS, 2); echo $p->format(0.12345), "\n";
$f = new NumberFormatter("en", 1); echo $f->format(5, NumberFormatter::TYPE_INT64), " ", $f->format(5.7, NumberFormatter::TYPE_INT32), " ", $f->format("12"), " ", $f->format(true), "\n";
var_dump($f->parse("1,234.5"), (new NumberFormatter("de", 1))->parse("1.234,5"), $f->parse("abc"), $f->parse("12abc"), $f->parse("1,234.5", NumberFormatter::TYPE_INT32), $f->parse("-42"));
$pos = 2; var_dump($f->parse("xx3.5yy", NumberFormatter::TYPE_DOUBLE, $pos), $pos);
var_dump((new NumberFormatter("en", 3))->parse("25%"), (new NumberFormatter("en_US", 2))->parseCurrency("$1,234.50", $cur), $cur);
echo intl_get_error_code(), " ", intl_get_error_message(), "\n";
for ($i = 0; $i <= 19; $i++) echo var_export($f->getAttribute($i), true), " "; echo "\n";
var_dump($f->setAttribute(999, 1), $f->getErrorCode(), $f->getErrorMessage(), $f->getAttribute(999), intl_get_error_message());
var_dump($f->getTextAttribute(NumberFormatter::POSITIVE_PREFIX), $f->getTextAttribute(NumberFormatter::NEGATIVE_PREFIX), (new NumberFormatter("en_US", 2))->getTextAttribute(NumberFormatter::POSITIVE_PREFIX), $f->getTextAttribute(NumberFormatter::CURRENCY_CODE), $f->getSymbol(NumberFormatter::GROUPING_SEPARATOR_SYMBOL), (new NumberFormatter("de", 2))->getSymbol(NumberFormatter::CURRENCY_SYMBOL));
try { new NumberFormatter("zz", 1); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { new NumberFormatter("en", 99); } catch (IntlException $e) { echo $e->getMessage(), "\n"; }
var_dump(NumberFormatter::create("en", 99), intl_get_error_code(), intl_get_error_message());
echo numfmt_format(numfmt_create("de_DE", NumberFormatter::DECIMAL), 1234.5), "\n";
var_dump(Locale::canonicalize("en-us"), Locale::getPrimaryLanguage("sr_Latn_RS"), Locale::getScript("sr_Latn_RS"), Locale::getRegion("sr_Latn_RS"), Locale::parseLocale("zh_Hant_TW_POSIX"), Locale::composeLocale(["language" => "de", "region" => "AT"]), Locale::acceptFromHttp("fr-CH, fr;q=0.9, en;q=0.8"), Locale::isRightToLeft("ar"), Locale::getKeywords("de_DE@currency=EUR;collation=phonebook"));
