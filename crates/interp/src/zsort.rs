//! PHP's sort algorithm (Zend/zend_sort.c): insertion sort up to 16
//! elements, else a quicksort with median-of-3/5 pivots; made stable by
//! falling back to the original order on ties, as PHP 8 does. Ported
//! exactly so comparators that are not a consistent order (returning -1
//! for equal elements, say) put elements where PHP puts them.

use std::cmp::Ordering;

/// Sort `v` like PHP's sort()/usort() with comparator `cmp`.
pub fn sort<T>(v: &mut Vec<T>, mut cmp: impl FnMut(&T, &T) -> Ordering) {
    let mut items: Vec<(usize, T)> = std::mem::take(v).into_iter().enumerate().collect();
    {
        let mut gt = |a: &(usize, T), b: &(usize, T)| match cmp(&a.1, &b.1) {
            Ordering::Equal => a.0 > b.0,
            o => o == Ordering::Greater,
        };
        let n = items.len();
        zend_sort(&mut items, 0, n, &mut gt);
    }
    v.extend(items.into_iter().map(|(_, t)| t));
}

type Gt<'a, T> = dyn FnMut(&T, &T) -> bool + 'a;

fn gt<T>(v: &[T], f: &mut Gt<T>, a: usize, b: usize) -> bool {
    f(&v[a], &v[b])
}

fn sort2<T>(v: &mut [T], f: &mut Gt<T>, a: usize, b: usize) {
    if gt(v, f, a, b) {
        v.swap(a, b);
    }
}

fn sort3<T>(v: &mut [T], f: &mut Gt<T>, a: usize, b: usize, c: usize) {
    if !gt(v, f, a, b) {
        if !gt(v, f, b, c) {
            return;
        }
        v.swap(b, c);
        if gt(v, f, a, b) {
            v.swap(a, b);
        }
        return;
    }
    if !gt(v, f, c, b) {
        v.swap(a, c);
        return;
    }
    v.swap(a, b);
    if gt(v, f, b, c) {
        v.swap(b, c);
    }
}

fn sort4<T>(v: &mut [T], f: &mut Gt<T>, a: usize, b: usize, c: usize, d: usize) {
    sort3(v, f, a, b, c);
    if gt(v, f, c, d) {
        v.swap(c, d);
        if gt(v, f, b, c) {
            v.swap(b, c);
            if gt(v, f, a, b) {
                v.swap(a, b);
            }
        }
    }
}

fn sort5<T>(v: &mut [T], f: &mut Gt<T>, a: usize, b: usize, c: usize, d: usize, e: usize) {
    sort4(v, f, a, b, c, d);
    if gt(v, f, d, e) {
        v.swap(d, e);
        if gt(v, f, c, d) {
            v.swap(c, d);
            if gt(v, f, b, c) {
                v.swap(b, c);
                if gt(v, f, a, b) {
                    v.swap(a, b);
                }
            }
        }
    }
}

/// zend_insert_sort over v[start..start+n].
fn insert_sort<T>(v: &mut [T], start: usize, n: usize, f: &mut Gt<T>) {
    let s = start;
    match n {
        0 | 1 => {}
        2 => sort2(v, f, s, s + 1),
        3 => sort3(v, f, s, s + 1, s + 2),
        4 => sort4(v, f, s, s + 1, s + 2, s + 3),
        5 => sort5(v, f, s, s + 1, s + 2, s + 3, s + 4),
        _ => {
            let end = s + n;
            let sentry = s + 6;
            for i in s + 1..sentry {
                let mut j = i - 1;
                if !gt(v, f, j, i) {
                    continue;
                }
                while j != s {
                    j -= 1;
                    if !gt(v, f, j, i) {
                        j += 1;
                        break;
                    }
                }
                let mut k = i;
                while k > j {
                    v.swap(k, k - 1);
                    k -= 1;
                }
            }
            for i in sentry..end {
                let mut j = i - 1;
                if !gt(v, f, j, i) {
                    continue;
                }
                loop {
                    j -= 2;
                    if !gt(v, f, j, i) {
                        j += 1;
                        if !gt(v, f, j, i) {
                            j += 1;
                        }
                        break;
                    }
                    if j == s {
                        break;
                    }
                    if j == s + 1 {
                        j -= 1;
                        if gt(v, f, i, j) {
                            j += 1;
                        }
                        break;
                    }
                }
                let mut k = i;
                while k > j {
                    v.swap(k, k - 1);
                    k -= 1;
                }
            }
        }
    }
}

fn zend_sort<T>(v: &mut [T], mut base: usize, mut n: usize, f: &mut Gt<T>) {
    loop {
        if n <= 16 {
            insert_sort(v, base, n, f);
            return;
        }
        let start = base;
        let end = start + n;
        let offset = n >> 1;
        let mut pivot = start + offset;
        if n >> 10 != 0 {
            let delta = offset >> 1;
            sort5(v, f, start, start + delta, pivot, pivot + delta, end - 1);
        } else {
            sort3(v, f, start, pivot, end - 1);
        }
        v.swap(start + 1, pivot);
        pivot = start + 1;
        let mut i = pivot + 1;
        let mut j = end - 1;
        'part: loop {
            while gt(v, f, pivot, i) {
                i += 1;
                if i == j {
                    break 'part;
                }
            }
            j -= 1;
            if j == i {
                break 'part;
            }
            while gt(v, f, j, pivot) {
                j -= 1;
                if j == i {
                    break 'part;
                }
            }
            v.swap(i, j);
            i += 1;
            if i == j {
                break 'part;
            }
        }
        v.swap(pivot, i - 1);
        if (i - 1) - start < end - i {
            zend_sort(v, start, (i - start) - 1, f);
            base = i;
            n = end - i;
        } else {
            zend_sort(v, i, end - i, f);
            n = (i - start) - 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_like_a_stable_sort() {
        for n in [0usize, 1, 2, 5, 6, 7, 16, 17, 100, 2000] {
            let mut v: Vec<u64> = (0..n as u64).map(|i| (i * 7919) % 31).collect();
            let mut want = v.clone();
            want.sort();
            sort(&mut v, |a, b| a.cmp(b));
            assert_eq!(v, want, "n={n}");
        }
    }

    #[test]
    fn keeps_order_with_always_less() {
        let mut v = vec!["flex", "mx-2", "max-w-150"];
        sort(&mut v, |_, _| Ordering::Less);
        assert_eq!(v, ["flex", "mx-2", "max-w-150"]);
    }
}
