<?php
date_default_timezone_set('Europe/Berlin');
$ts = 1700000000;
var_dump(@strftime('%Y-%m-%d %H:%M:%S %a %b %j %u %w %%', $ts), @gmstrftime('%Y-%m-%d %H:%M:%S', $ts), @strftime('%A %B %e %I %p %y', 1720000000));
var_dump(timezone_name_from_abbr("EST"), timezone_name_from_abbr("", 3600, 0), timezone_name_from_abbr("", -18000, 1), timezone_name_from_abbr("CET"), timezone_name_from_abbr("cst", 28800), timezone_name_from_abbr("foo"), timezone_name_from_abbr("utc"), timezone_name_from_abbr("a"));
var_dump(date_sun_info(1700000000, 52.52, 13.405));
var_dump(date_sun_info(1718800000, 78.22, 15.65));
var_dump(date_sun_info(1703000000, 78.22, 15.65));
var_dump(@date_sunrise($ts, SUNFUNCS_RET_STRING, 52.52, 13.405), @date_sunset($ts, SUNFUNCS_RET_TIMESTAMP, 52.52, 13.405), round(@date_sunrise($ts, SUNFUNCS_RET_DOUBLE, 52.52, 13.405, 90.83, 1), 10));
date_default_timezone_set('America/New_York');
var_dump(date_sun_info(1700000000, 40.71, -74.0));
var_dump(date_parse_from_format("Y-m-d", "2024-01-15"));
var_dump(date_parse_from_format("j.n.Y H:i", "6.1.2009 13:05"));
var_dump(date_parse_from_format("Y-m-d", "nope")['error_count'] > 0);
