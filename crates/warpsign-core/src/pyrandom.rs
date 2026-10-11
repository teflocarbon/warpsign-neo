//! A bit-for-bit port of CPython's `random.Random`, limited to what the ID
//! encoding needs: seeding from a string, `random()` and `choices()`.
//!
//! Python warpsign derives encoded bundle IDs from `random.Random(seed)`, so
//! the same seed must produce the same characters here. Any drift would give
//! re-signed apps a different bundle ID than the one already registered.

use sha2::{Digest, Sha512};

const N: usize = 624;
const M: usize = 397;
const MATRIX_A: u32 = 0x9908_b0df;
const UPPER_MASK: u32 = 0x8000_0000;
const LOWER_MASK: u32 = 0x7fff_ffff;

/// CPython's Mersenne Twister (`Modules/_randommodule.c`)
pub struct PyRandom {
    state: [u32; N],
    index: usize,
}

impl PyRandom {
    /// Equivalent to `random.Random(seed)` with a `str` seed (seeding version 2)
    pub fn from_str_seed(seed: &str) -> Self {
        // random.py: a = int.from_bytes(a.encode() + sha512(a.encode()).digest(), "big")
        let mut big_endian = seed.as_bytes().to_vec();
        big_endian.extend_from_slice(&Sha512::digest(seed.as_bytes()));

        // _randommodule.c: split abs(a) into 32-bit words, least significant first
        let significant = big_endian
            .iter()
            .position(|&b| b != 0)
            .map_or(&[][..], |start| &big_endian[start..]);
        let mut key: Vec<u32> = significant
            .rchunks(4)
            .map(|chunk| {
                chunk
                    .iter()
                    .fold(0u32, |word, &b| (word << 8) | u32::from(b))
            })
            .collect();
        if key.is_empty() {
            key.push(0);
        }

        let mut rng = Self {
            state: [0; N],
            index: N,
        };
        rng.init_by_array(&key);
        rng
    }

    fn init_genrand(&mut self, seed: u32) {
        self.state[0] = seed;
        for i in 1..N {
            let prev = self.state[i - 1];
            self.state[i] = 1_812_433_253u32
                .wrapping_mul(prev ^ (prev >> 30))
                .wrapping_add(i as u32);
        }
        self.index = N;
    }

    fn init_by_array(&mut self, key: &[u32]) {
        self.init_genrand(19_650_218);
        let mut i = 1;
        let mut j = 0;
        for _ in 0..N.max(key.len()) {
            let prev = self.state[i - 1];
            self.state[i] = (self.state[i] ^ (prev ^ (prev >> 30)).wrapping_mul(1_664_525))
                .wrapping_add(key[j])
                .wrapping_add(j as u32);
            i += 1;
            j += 1;
            if i >= N {
                self.state[0] = self.state[N - 1];
                i = 1;
            }
            if j >= key.len() {
                j = 0;
            }
        }
        for _ in 0..N - 1 {
            let prev = self.state[i - 1];
            self.state[i] = (self.state[i] ^ (prev ^ (prev >> 30)).wrapping_mul(1_566_083_941))
                .wrapping_sub(i as u32);
            i += 1;
            if i >= N {
                self.state[0] = self.state[N - 1];
                i = 1;
            }
        }
        self.state[0] = 0x8000_0000;
    }

    fn next_u32(&mut self) -> u32 {
        if self.index >= N {
            for kk in 0..N {
                let y = (self.state[kk] & UPPER_MASK) | (self.state[(kk + 1) % N] & LOWER_MASK);
                let mag = if y & 1 == 1 { MATRIX_A } else { 0 };
                self.state[kk] = self.state[(kk + M) % N] ^ (y >> 1) ^ mag;
            }
            self.index = 0;
        }
        let mut y = self.state[self.index];
        self.index += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c_5680;
        y ^= (y << 15) & 0xefc6_0000;
        y ^= y >> 18;
        y
    }

    /// Equivalent to `random()`: a float in [0, 1) with 53 bits of precision
    pub fn random(&mut self) -> f64 {
        let a = self.next_u32() >> 5;
        let b = self.next_u32() >> 6;
        (f64::from(a) * 67_108_864.0 + f64::from(b)) * (1.0 / 9_007_199_254_740_992.0)
    }

    /// Equivalent to `choices(population, k=k)` without weights
    pub fn choices(&mut self, population: &[char], k: usize) -> String {
        let n = population.len() as f64;
        (0..k)
            .map(|_| population[(self.random() * n).floor() as usize])
            .collect()
    }
}
