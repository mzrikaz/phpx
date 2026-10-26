<?php
echo md5(implode(",", timezone_identifiers_list())), " ", count(timezone_identifiers_list()), "\n";
echo md5(implode(",", DateTimeZone::listIdentifiers(DateTimeZone::ALL_WITH_BC))), "\n";
echo implode(",", DateTimeZone::listIdentifiers(DateTimeZone::PER_COUNTRY, "LK")), " ", implode(",", DateTimeZone::listIdentifiers(DateTimeZone::PER_COUNTRY, "au")), "\n";
echo count(DateTimeZone::listIdentifiers(DateTimeZone::EUROPE | DateTimeZone::UTC)), " ", implode(",", timezone_identifiers_list(DateTimeZone::UTC)), "\n";
try { DateTimeZone::listIdentifiers(DateTimeZone::PER_COUNTRY, "XYZ"); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
foreach (DateTimeZone::listIdentifiers() as $id) { try { new DateTimeZone($id); } catch (Exception $e) { echo "bad $id\n"; } }
