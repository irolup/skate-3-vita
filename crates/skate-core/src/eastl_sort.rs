//! The executable's sort (EASTL's `sort`, as retail TU3 runs it; research of 7 Nov 2026): an introsort that is not
//! stable, so where equal keys end up depends on the algorithm, and lists the player sees (the shop's brand order,
//! 826035F0) must be sorted by this exact procedure to come out in retail's order.
//!
//! - 82630CF0: depth limit `2 * floor(log2(n))`; the introsort loop; then an insertion sort over the first 28 and an
//!   unguarded insertion of the rest (only when there are more than 28).
//! - 82631A60: while a range holds more than 28 and the depth is not spent: the pivot is the median of the first, the
//!   middle and the last (in that comparison order), a Hoare partition around a copy of it, the right part recursed
//!   with one less depth and the left part looped on. Leaving with the depth at exactly 0 sorts the range by heap
//!   (82632810), short or not.
//! - 82632810 / 82632EE8: make-heap from the last parent down, then pop each top to the end.

/// Retail's insertion threshold (0xE1 bytes of 8-byte elements: ranges of at most 28 are left to the insertion sort).
const THRESHOLD: usize = 28;

/// Sorts `v` as 82630CF0 does, `less(a, b)` meaning a goes before b.
pub fn sort<T: Clone>(v: &mut [T], less: &impl Fn(&T, &T) -> bool) {
    let n = v.len();
    if n == 0 {
        return;
    }
    let mut bits = 0usize;
    let mut i = n;
    while i != 0 {
        bits += 1;
        i >>= 1;
    }
    introsort(v, 0, n, 2 * bits as isize - 2, less);
    if n <= THRESHOLD {
        insertion(v, 0, n, less);
    } else {
        insertion(v, 0, THRESHOLD, less);
        for i in THRESHOLD..n {
            let value = v[i].clone();
            let mut j = i;
            while less(&value, &v[j - 1]) {
                v[j] = v[j - 1].clone();
                j -= 1;
            }
            v[j] = value;
        }
    }
}

/// 82631A60 on `v[first..last]`.
fn introsort<T: Clone>(v: &mut [T], first: usize, mut last: usize, mut depth: isize, less: &impl Fn(&T, &T) -> bool) {
    while last - first > THRESHOLD && depth >= 1 {
        let mid = first + (last - first) / 2;
        let end = last - 1;
        let pick = if less(&v[first], &v[mid]) {
            if less(&v[mid], &v[end]) {
                mid
            } else if less(&v[first], &v[end]) {
                end
            } else {
                first
            }
        } else if less(&v[first], &v[end]) {
            first
        } else if less(&v[mid], &v[end]) {
            end
        } else {
            mid
        };
        let pivot = v[pick].clone();
        let (mut lo, mut hi) = (first, last);
        loop {
            while less(&v[lo], &pivot) {
                lo += 1;
            }
            hi -= 1;
            while less(&pivot, &v[hi]) {
                hi -= 1;
            }
            if hi <= lo {
                break;
            }
            v.swap(lo, hi);
            lo += 1;
        }
        depth -= 1;
        introsort(v, lo, last, depth, less);
        last = lo;
    }
    // Retail sorts by heap whenever it leaves with the depth spent, however short the range.
    if depth == 0 {
        heap_sort(v, first, last, less);
    }
}

/// 82631C38: a guarded insertion sort.
fn insertion<T: Clone>(v: &mut [T], first: usize, last: usize, less: &impl Fn(&T, &T) -> bool) {
    for i in first + 1..last {
        let value = v[i].clone();
        let mut j = i;
        while j != first && less(&value, &v[j - 1]) {
            v[j] = v[j - 1].clone();
            j -= 1;
        }
        v[j] = value;
    }
}

/// 82632810 called with middle == last: make a heap, then pop every top to the end.
fn heap_sort<T: Clone>(v: &mut [T], first: usize, last: usize, less: &impl Fn(&T, &T) -> bool) {
    let h = &mut v[first..last];
    let n = h.len();
    if n > 1 {
        let mut parent = (n - 2) / 2 + 1;
        while parent != 0 {
            parent -= 1;
            let value = h[parent].clone();
            adjust(h, parent, n, value, less);
        }
    }
    let mut len = n;
    while len > 1 {
        let value = h[len - 1].clone();
        h[len - 1] = h[0].clone();
        len -= 1;
        adjust(h, 0, len, value, less);
    }
}

/// 82632EE8 (top = hole): sift the hole down to a leaf by the larger child, then push `value` back up.
fn adjust<T: Clone>(h: &mut [T], top: usize, len: usize, value: T, less: &impl Fn(&T, &T) -> bool) {
    let mut hole = top;
    let mut child = 2 * (hole + 1);
    while child < len {
        if less(&h[child], &h[child - 1]) {
            child -= 1;
        }
        h[hole] = h[child].clone();
        hole = child;
        child = 2 * (child + 1);
    }
    if child == len {
        h[hole] = h[child - 1].clone();
        hole = child - 1;
    }
    while hole > top {
        let parent = (hole - 1) / 2;
        if !less(&h[parent], &value) {
            break;
        }
        h[hole] = h[parent].clone();
        hole = parent;
    }
    h[hole] = value;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lcg(seed: &mut u32) -> u32 {
        *seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        *seed >> 16
    }

    #[test]
    fn it_sorts_every_size_by_key() {
        let mut seed = 7;
        for n in 0..300 {
            let mut v: Vec<(u32, usize)> = (0..n).map(|i| (lcg(&mut seed) % 9, i)).collect();
            sort(&mut v, &|a, b| a.0 < b.0);
            assert!(v.windows(2).all(|w| w[0].0 <= w[1].0), "n {n}");
            let mut ids: Vec<usize> = v.iter().map(|e| e.1).collect();
            ids.sort_unstable();
            assert_eq!(ids, (0..n).collect::<Vec<_>>(), "a permutation");
        }
    }

    #[test]
    fn small_ranges_are_an_insertion_sort_so_equal_keys_keep_their_order() {
        let mut v = vec![(2, 'a'), (1, 'b'), (2, 'c'), (1, 'd'), (0, 'e')];
        sort(&mut v, &|a, b| a.0 < b.0);
        assert_eq!(v, [(0, 'e'), (1, 'b'), (1, 'd'), (2, 'a'), (2, 'c')]);
    }

    #[test]
    fn large_ranges_partition_and_reorder_equal_keys() {
        // 40 equal keys and one smaller at the end: the partition swaps, so the equal keys do not stay in order.
        let mut v: Vec<(u32, usize)> = (0..40).map(|i| (1, i)).collect();
        v.push((0, 40));
        sort(&mut v, &|a, b| a.0 < b.0);
        assert_eq!(v[0], (0, 40));
        assert_ne!(v[1..].iter().map(|e| e.1).collect::<Vec<_>>(), (0..40).collect::<Vec<_>>());
    }

    #[test]
    fn the_heap_sort_sorts() {
        let mut seed = 3;
        let mut v: Vec<u32> = (0..100).map(|_| lcg(&mut seed) % 50).collect();
        heap_sort(&mut v, 0, 100, &|a, b| a < b);
        assert!(v.windows(2).all(|w| w[0] <= w[1]));
    }
}
