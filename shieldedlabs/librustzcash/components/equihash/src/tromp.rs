//! Rust interface to the tromp equihash solver.

use std::marker::{PhantomData, PhantomPinned};
use std::slice;
use std::vec::Vec;

use blake2b_simd::State;

use crate::{blake2b, minimal::minimal_from_indices, params::Params, verify};

#[repr(C)]
struct CEqui {
    _f: [u8; 0],
    _m: PhantomData<(*mut u8, PhantomPinned)>,
}

#[link(name = "equitromp")]
extern "C" {
    #[allow(improper_ctypes)]
    fn equi_new(
        blake2b_clone: extern "C" fn(state: *const State) -> *mut State,
        blake2b_free: extern "C" fn(state: *mut State),
        blake2b_update: extern "C" fn(state: *mut State, input: *const u8, input_len: usize),
        blake2b_finalize: extern "C" fn(state: *mut State, output: *mut u8, output_len: usize),
    ) -> *mut CEqui;
    fn equi_free(eq: *mut CEqui);
    #[allow(improper_ctypes)]
    fn equi_setstate(eq: *mut CEqui, ctx: *const State);
    fn equi_clearslots(eq: *mut CEqui);
    fn equi_digit0(eq: *mut CEqui, id: u32);
    fn equi_digitodd(eq: *mut CEqui, r: u32, id: u32);
    fn equi_digiteven(eq: *mut CEqui, r: u32, id: u32);
    fn equi_digitK(eq: *mut CEqui, id: u32);
    fn equi_nsols(eq: *const CEqui) -> usize;
    /// Returns `equi_nsols()` solutions of length `2^K`, in a single memory allocation.
    fn equi_sols(eq: *const CEqui) -> *const u32;
}

/// Performs a single equihash solver run with equihash parameters `p` and hash state `curr_state`.
/// Returns zero or more unique solutions.
///
/// # SAFETY
///
/// The parameters to this function must match the hard-coded parameters in the C++ code.
///
/// This function uses unsafe code for FFI into the tromp solver.
#[allow(unsafe_code)]
#[allow(clippy::print_stdout)]
unsafe fn worker(eq: *mut CEqui, p: Params, curr_state: &State) -> Vec<Vec<u32>> {
    // SAFETY: caller must supply a valid `eq` instance.
    //
    // Review Note: nsols is set to zero in C++ here
    equi_setstate(eq, curr_state);

    // Initialization done, start algo driver.
    equi_digit0(eq, 0);
    equi_clearslots(eq);
    // SAFETY: caller must supply a `p` instance that matches the hard-coded values in the C code.
    for r in 1..p.k {
        if (r & 1) != 0 {
            equi_digitodd(eq, r, 0)
        } else {
            equi_digiteven(eq, r, 0)
        };
        equi_clearslots(eq);
    }
    // Review Note: nsols is increased here, but only if the solution passes the strictly ordered check.
    // With 256 nonces, we get to around 6/9 digits strictly ordered.
    equi_digitK(eq, 0);

    let solutions = {
        let nsols = equi_nsols(eq);
        let sols = equi_sols(eq);
        let solution_len = 1 << p.k;
        //println!("{nsols} solutions of length {solution_len} at {sols:?}");

        // SAFETY:
        // - caller must supply a `p` instance that matches the hard-coded values in the C code.
        // - `sols` is a single allocation containing at least `nsols` solutions.
        // - this slice is a shared ref to the memory in a valid `eq` instance supplied by the caller.
        let solutions: &[u32] = slice::from_raw_parts(sols, nsols * solution_len);

        /*
        println!(
            "{nsols} solutions of length {solution_len} as a slice of length {:?}",
            solutions.len()
        );
        */

        let mut chunks = solutions.chunks_exact(solution_len);

        // SAFETY:
        // - caller must supply a `p` instance that matches the hard-coded values in the C code.
        // - each solution contains `solution_len` u32 values.
        // - the temporary slices are shared refs to a valid `eq` instance supplied by the caller.
        // - the bytes in the shared ref are copied before they are returned.
        // - dropping `solutions: &[u32]` does not drop the underlying memory owned by `eq`.
        let mut solutions = (&mut chunks)
            .map(|solution| solution.to_vec())
            .collect::<Vec<_>>();

        assert_eq!(chunks.remainder().len(), 0);

        // Sometimes the solver returns identical solutions.
        solutions.sort();
        solutions.dedup();

        solutions
    };

    /*
    println!(
        "{} solutions as cloned vectors of length {:?}",
        solutions.len(),
        solutions
            .iter()
            .map(|solution| solution.len())
            .collect::<Vec<_>>()
    );
    */

    solutions
}

/// Performs multiple equihash solver runs with equihash parameters `200, 9`, initialising the hash with
/// the supplied partial `input`. Between each run, generates a new nonce of length `N` using the
/// `next_nonce` function.
///
/// Returns zero or more unique solutions.
fn solve_200_9_uncompressed<const N: usize>(
    input: &[u8],
    mut next_nonce: impl FnMut() -> Option<[u8; N]>,
) -> Vec<Vec<u32>> {
    let p = Params::new(200, 9).expect("should be valid");
    let mut state = verify::initialise_state(p.n, p.k, p.hash_output());
    state.update(input);

    // Create solver and initialize it.
    //
    // # SAFETY
    // - the parameters 200,9 match the hard-coded parameters in the C++ code.
    // - tromp is compiled without multi-threading support, so each instance can only support 1 thread.
    // - the blake2b functions are in the correct order in Rust and C++ initializers.
    #[allow(unsafe_code)]
    let eq = unsafe {
        equi_new(
            blake2b::blake2b_clone,
            blake2b::blake2b_free,
            blake2b::blake2b_update,
            blake2b::blake2b_finalize,
        )
    };

    let solutions = loop {
        let nonce = match next_nonce() {
            Some(nonce) => nonce,
            None => break vec![],
        };

        let mut curr_state = state.clone();
        // Review Note: these hashes are changing when the nonce changes
        curr_state.update(&nonce);

        // SAFETY:
        // - the parameters 200,9 match the hard-coded parameters in the C++ code.
        // - the eq instance is initilized above.
        #[allow(unsafe_code)]
        let solutions = unsafe { worker(eq, p, &curr_state) };
        if !solutions.is_empty() {
            break solutions;
        }
    };

    // SAFETY:
    // - the eq instance is initilized above, and not used after this point.
    #[allow(unsafe_code)]
    unsafe {
        equi_free(eq)
    };

    solutions
}

/// Performs multiple equihash solver runs with equihash parameters `200, 9`, initialising the hash with
/// the supplied partial `input`. Between each run, generates a new nonce of length `N` using the
/// `next_nonce` function.
///
/// Returns zero or more unique compressed solutions.
pub fn solve_200_9<const N: usize>(
    input: &[u8],
    next_nonce: impl FnMut() -> Option<[u8; N]>,
) -> Vec<Vec<u8>> {
    let p = Params::new(200, 9).expect("should be valid");
    let solutions = solve_200_9_uncompressed(input, next_nonce);

    let mut solutions: Vec<Vec<u8>> = solutions
        .iter()
        .map(|solution| minimal_from_indices(p, solution))
        .collect();

    // Just in case the solver returns solutions that become the same when compressed.
    solutions.sort();
    solutions.dedup();

    solutions
}

#[cfg(test)]
mod tests {
    use std::println;

    use super::solve_200_9;

    #[test]
    #[allow(clippy::print_stdout)]
    fn run_solver() {
        let input = b"Equihash is an asymmetric PoW based on the Generalised Birthday problem.";
        let mut nonce: [u8; 32] = [
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0,
        ];
        let mut nonces = 0..=32_u32;
        let nonce_count = nonces.clone().count();

        let solutions = solve_200_9(input, || {
            let variable_nonce = nonces.next()?;
            println!("Using variable nonce [0..4] of {}", variable_nonce);

            let variable_nonce = variable_nonce.to_le_bytes();
            nonce[0] = variable_nonce[0];
            nonce[1] = variable_nonce[1];
            nonce[2] = variable_nonce[2];
            nonce[3] = variable_nonce[3];

            Some(nonce)
        });

        if solutions.is_empty() {
            // Expected solution rate is documented at:
            // https://github.com/tromp/equihash/blob/master/README.md
            panic!("Found no solutions after {nonce_count} runs, expected 1.88 solutions per run",);
        } else {
            println!("Found {} solutions:", solutions.len());
            for (sol_num, solution) in solutions.iter().enumerate() {
                println!("Validating solution {sol_num}:-\n{}", hex::encode(solution));
                crate::is_valid_solution(200, 9, input, &nonce, solution).unwrap_or_else(|error| {
                    panic!(
                        "unexpected invalid equihash 200, 9 solution:\n\
                             error: {error:?}\n\
                             input: {input:?}\n\
                             nonce: {nonce:?}\n\
                             solution: {solution:?}"
                    )
                });
                println!("Solution {sol_num} is valid!\n");
            }
        }
    }
}
