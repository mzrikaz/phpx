<?php
date_default_timezone_set('UTC');
$base = 1705314645; // 2024-01-15 10:30:45 UTC, Monday
foreach (['now', 'today', 'tomorrow', 'yesterday', 'midnight', 'noon', '+1 day', '-2 days', '+1 week 2 days', '1 month ago', '+1 month', 'next monday', 'last friday', 'monday', 'sunday',
  'next month', 'last year', 'first day of next month', 'last day of this month', 'first day of january 2025', 'last day of feb', 'first monday of next month', 'last friday of january 2024',
  '2024-03-10', '2024-03-10 14:15', '2024-03-10T14:15:16Z', '2024-03-10T14:15:16+05:30', '2024-03-10 14:15:16.250000', '10 September 2000', 'September 10, 2000', 'Sep 10 2000',
  '15-Jan-2024', '01/02/2024', '02.01.2024', '20240102', '@86400', '10am', '3:30pm', 'tomorrow 9:00', 'noon tomorrow', '2024-01-31 +1 month', 'Sat, 01 Jan 2022 10:00:00 +0000',
  '2024-W03-2', '+3 weekdays', 'next week', 'this week', '1 year 2 months ago', 'January', 'jan 2023', '2024-02', '1530', 'garbage', '', 'Europe/Paris', '2024-01-01 Europe/Paris', '2024-06-01 12:00 EST'] as $s) {
    $r = strtotime($s, $base);
    printf("%-34s %s\n", $s, $r === false ? 'false' : date('Y-m-d H:i:s', $r));
}
$d = new DateTime('2024-01-15 10:30:45.123456', new DateTimeZone('Europe/Paris'));
echo $d->format('Y-m-d H:i:s.u T P O e I U N jS F l D z t L o W G g A a v B c r'), "\n";
echo $d->format(DATE_ATOM), ' ', $d->format(DateTime::RFC2822), ' ', $d->format(DateTimeInterface::COOKIE), "\n";
$i = new DateTimeImmutable('2024-03-30 12:00', new DateTimeZone('Europe/London'));
$j = $i->modify('+1 day'); echo $i->format('c'), ' ', $j->format('c T'), "\n";
echo $i->add(new DateInterval('P1M2DT3H'))->format('c'), ' ', $i->sub(new DateInterval('PT90M'))->format('c'), "\n";
$a = new DateTime('2024-01-31 10:00'); $b = new DateTime('2025-03-02 08:15:30');
$df = $a->diff($b); echo $df->format('%R %y y %m m %d d %h h %i i %s s, %a days'), ' ', $df->days, ' ', $df->invert, "\n";
$df2 = $b->diff($a); echo $df2->format('%R%a %Y-%M-%D %H:%I:%S'), "\n";
var_dump($a < $b, $a == clone $a, max($a, $b)->format('Y'));
$c = DateTime::createFromFormat('d/m/Y H:i', '15/08/2023 14:30'); echo $c->format('c'), "\n";
$c = DateTime::createFromFormat('!Y-m-d', '2023-08-15'); echo $c->format('c u'), "\n";
$c = DateTimeImmutable::createFromFormat('Y-m-d H:i:s.u P', '2023-08-15 01:02:03.456789 +02:00'); echo $c->format('c u e'), "\n";
var_dump(DateTime::createFromFormat('Y-m-d', 'nope'));
$t = new DateTime('2024-05-05 05:05:05'); $t->setDate(2020, 2, 30)->setTime(25, 61, 61); echo $t->format('c'), "\n";
$t->setTimezone(new DateTimeZone('America/New_York')); echo $t->format('c T'), ' ', $t->getOffset(), ' ', $t->getTimezone()->getName(), "\n";
$t->setTimestamp(0); echo $t->format('c'), ' ', $t->getTimestamp(), "\n";
$t->setISODate(2024, 1, 1); echo $t->format('Y-m-d l'), "\n";
echo (new DateTime('@1700000000'))->format('c e'), "\n";
echo json_encode(new DateTime('2020-01-01 00:00:00')), "\n";
$s = serialize(new DateTimeImmutable('2021-02-03 04:05:06', new DateTimeZone('Asia/Tokyo'))); echo $s, "\n"; echo unserialize($s)->format('c'), "\n";
$iv = new DateInterval('P1Y2M10DT2H30M'); echo $iv->format('%y-%m-%d %h:%i:%s %a'), "\n"; print_r($iv);
$p = new DatePeriod(new DateTime('2024-01-01'), new DateInterval('P1W'), new DateTime('2024-02-01'));
foreach ($p as $x) echo $x->format('m/d '), ''; echo "\n";
foreach (new DatePeriod(new DateTimeImmutable('2024-01-01'), new DateInterval('P1D'), 3) as $x) echo $x->format('D '); echo "\n";
try { new DateTime('not a date'); } catch (Exception $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { new DateTimeZone('Mars/Base'); } catch (Exception $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { new DateInterval('X'); } catch (Exception $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
echo date('D, d M Y', 0), ' ', gmdate('H:i', 3600), ' ', mktime(0, 0, 0, 2, 30, 2024), ' ', gmmktime(12, 0, 0, 1, 1, 70), "\n";
date_default_timezone_set('America/Los_Angeles'); echo date_default_timezone_get(), ' ', date('Y-m-d H:i T', $base), ' ', mktime(0,0,0,1,1,2024), "\n";
echo idate('Y', $base), ' ', implode(',', getdate($base)), "\n";
var_dump(checkdate(2, 29, 2023), date_create('bogus'));
$x = new DateTime('2024-03-10 01:30:00'); $x->modify('+1 hour'); echo $x->format('c T'), "\n";
echo (new DateTime('2024-03-10 02:30:00'))->format('c T'), ' ', (new DateTime('2024-11-03 01:30:00'))->format('c T'), "\n";
print_r(new DateTimeZone("+05:30")); print_r(new DateTimeZone("EST"));
print_r(new DateTime('2020-02-02', new DateTimeZone('UTC')));
var_export(new DateTimeImmutable('2020-02-02 10:00', new DateTimeZone('+01:00'))); echo "\n";
