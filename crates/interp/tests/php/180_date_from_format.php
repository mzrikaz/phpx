<?php
date_default_timezone_set('UTC');
$cases = [
  ['Y-m-d H:i:s', '2026-10-26T06:21:33.000000Z'], ['Y-m-d H:i:s', '2026-10-26 06:21:33'], ['Y-m-d H:i:s', '2026-10-26 06:21:33 extra'],
  ['Y-m-d H:i:s', 'garbage'], ['Y-m-d', '2026-02-30'], ['!Y-m-d', '2026-10-26'], ['Y-m-d|', '2026-10-26'], ['d/m/Y H:i', '26/10/2026 6:21'],
  ['D, d M Y', 'Mon, 26 Oct 2026'], ['l jS F Y', 'Monday 26th October 2026'], ['U', '1700000000'], ['U.u', '1700000000.123456'],
  ['Y-m-d\TH:i:sP', '2026-10-26T06:21:33+05:30'], ['Y-m-d H:i:s T', '2026-10-26 06:21:33 EST'], ['Y-m-d H:i:s e', '2026-10-26 06:21:33 Asia/Colombo'],
  ['Y-m-d H:i:s.u', '2026-10-26 06:21:33.5'], ['Y-m-d H:i:s.v', '2026-10-26 06:21:33.12'], ['g:i a', '6:21 pm'], ['h:i A', '12:30 AM'], ['h:i', '13:30'],
  ['Y-m-d+', '2026-10-26 trailing'], ['Y-m-d H:i:s', '2026-10-26'], ['y-n-j', '26-1-5'], ['Y z', '2026 100'], ['z Y', '100 2026'], ['Y-m-d ?', '2026-10-26 X'],
  ['Y#m#d', '2026/10.26'], ['Y-m-d *', '2026-10-26 foo123'], ['H\h i\m', '06h 21m'], ['Y-m-d H:i:s', '2026-13-45 25:61:61'], ['Y-m-d G:i', '2026-10-26 6:05'],
  ['Ymd', '20261026'], ['YmdHis', '20261026062133'], ['Y-m-d', ''], ['Y-m-d H:i:s O', '2026-10-26 06:21:33 +0200'],
];
foreach ($cases as [$f, $t]) {
  $d = DateTime::createFromFormat($f, $t);
  echo json_encode([$f, $t]), ' => ', $d ? $d->format('Y-m-d') . (preg_match('/[HhGgisuvU!|]/', $f) ? $d->format(' H:i:s.u T P') : '') : 'false', ' ', json_encode(DateTime::getLastErrors()), "\n";
  $p = date_parse_from_format($f, $t); unset($p['zone_type']); echo '   ', json_encode($p), "\n";
}
new DateTime('2026-01-01'); var_dump(DateTime::getLastErrors());
echo (new DateTime('2026-10-26 06:21:33+05:30'))->format('T e P'), "\n";
echo (new DateTime('2026-10-26 06:21:33', new DateTimeZone('+05:30')))->format('T e P'), "\n";
echo DateTime::createFromFormat('Y-m-d H:i:sP', '2026-10-26 06:21:33+05:30')->format('T e P'), "\n";
echo DateTime::createFromFormat('U', '0')->format('T e P'), "\n";
echo (new DateTime('@0'))->format('T e P'), "\n";
var_dump(DateTime::createFromFormat('U', '0')->getTimezone()->getName());
