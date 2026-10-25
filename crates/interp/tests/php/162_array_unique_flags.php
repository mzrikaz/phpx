<?php
var_dump(array_unique([[1,2],[1,2],[3],"a"=>[3], 1, "1", 1.0, true, null, 0, "", "x", new stdClass], SORT_REGULAR));
var_dump(array_unique([3, "3", "a", "b", "a", 3.0, "03", " 3"]));
var_dump(array_unique([3, "3", "03", "3.0", "abc", "ABC"], SORT_NUMERIC));
var_dump(array_unique(["b", "a", "B", "a"], SORT_STRING|SORT_FLAG_CASE));
var_dump(array_unique([4, "4", "3", 4, 3, "3"], SORT_REGULAR));
