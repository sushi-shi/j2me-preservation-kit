//! The AMR-NB MR122 (12.2 kbit/s) speech decoder.
//!
//! This is a transliteration of the 3GPP TS 26.073 fixed-point decoder,
//! narrowed to the one mode the recovered corpus uses. It is integer-only, so
//! it produces identical samples on every target the port builds for —
//! native, WASM, and the site exporter alike — which a floating-point decoder
//! could not promise.
//!
//! # Why only MR122
//!
//! All 53 `.amr` entries across the surviving archives reduce to five unique
//! payloads, and every frame of all five is frame type 7 with its quality bit
//! set: no SID comfort noise, no `NO_DATA`, no other rate. Carrying the other
//! seven rates would mean carrying their codebooks and concealment tables
//! with nothing in the corpus to check them against, which is exactly the
//! kind of unverifiable code this repository refuses. [`super::decode`]
//! rejects any other frame type by name instead of guessing.
//!
//! # What the good-frame path leaves out
//!
//! The reference decoder interleaves speech decoding with error concealment,
//! comfort noise, and background-noise adaptation. For a stream of good
//! MR122 frames those are provably inert, and each is omitted for a stated
//! reason rather than by oversight:
//!
//! - Error concealment (`ec_gain_pitch`/`ec_gain_code` and the bad-frame
//!   gain attenuation) is reached only when `bfi != 0`. The corresponding
//!   `_update` calls do run on good frames, but with `bfi == 0` and
//!   `prev_bf == 0` they only write history that the concealment path reads.
//! - Phase dispersion is skipped by its own `mode != MR122` guard; only its
//!   closing excitation-mixing loop applies, and that loop is transliterated
//!   here.
//! - `Cb_gain_average`'s output is overwritten by `gain_code` for every mode
//!   above MR67, so its state is dead for MR122.
//! - Background-noise detection, excitation-energy control, and the
//!   excitation-energy history feed conditions that additionally require
//!   MR475, MR515, or MR59.
//! - The excitation-rescaling branch after synthesis is guarded by an
//!   overflow flag that is cleared immediately before `Syn_filt`, and the
//!   reference's `Syn_filt` never sets it. The branch cannot be taken.
//!
//! Their absence is not assumed safe — it is checked. The corpus test
//! compares every recovered blob against the fixed-point reference decoder
//! sample for sample, and any of these paths mattering would show up there.
//!
//! # What "the reference" names, exactly
//!
//! The oracle is opencore-amrnb 0.1.6, PacketVideo's fixed-point decoder
//! derived from the 3GPP TS 26.073 reference source and the implementation
//! behind Android's and most of the world's AMR playback. It is not
//! character-for-character the 3GPP source, and one difference is worth
//! naming rather than papering over: its `Syn_filt` saturates its output but
//! never raises the overflow flag, so the excitation-rescaling branch above
//! is dead there. A decoder built from the literal 3GPP source, whose basic
//! operators do raise that flag, could in principle take it on a loud enough
//! frame and diverge. Nothing in the corpus is that loud, and eight million
//! samples of randomly-parameterised speech did not reach it either, but the
//! claim this port makes is bit-exactness against opencore and it should be
//! read that way.

use super::fixed::{
    add, div_s, inv_sqrt, l_add, l_mac, l_mult, l_shl, l_shr, l_sub, log2, mult, negate, norm_l,
    pow2, round16, shl, sub,
};
use super::tables;

/// Samples in one 20 ms frame.
pub const FRAME_SAMPLES: usize = 160;

const L_SUBFR: usize = 40;
const SUBFRAMES: usize = 4;
const M: usize = 10;
const MP1: usize = M + 1;
const PIT_MIN_MR122: i16 = 18;
const PIT_MAX: i16 = 143;
const L_INTERPOL: usize = 11;
/// `old_exc`: the pitch history, the interpolation margin, and one subframe.
const EXC_HISTORY: usize = PIT_MAX as usize + L_INTERPOL;
const EXC_LEN: usize = EXC_HISTORY + L_SUBFR;
const LSF_GAP: i16 = 205;
const LSP_PRED_FAC_MR122: i16 = 21299;
const MEAN_ENER_MR122: i32 = 783_741;
const MIN_ENERGY_MR122: i16 = -2381;
/// Tilt-compensation factor (0.8).
const MU: i16 = 26214;
/// Adaptive gain-control factor (0.9).
const AGC_FAC: i16 = 29491;
/// Length of the post filter's impulse response.
const L_H: usize = 22;

const GAMMA3_MR122: [i16; M] = [22938, 16057, 11240, 7868, 5508, 3856, 2699, 1889, 1322, 925];
const GAMMA4_MR122: [i16; M] = [
    24576, 18432, 13824, 10368, 7776, 5832, 4374, 3281, 2461, 1846,
];
/// The output high-pass filter's numerator and denominator.
const HP_B: [i16; 3] = [7699, -15398, 7699];
const HP_A: [i16; 3] = [8192, 15836, -7667];

/// The decoder's persistent state: one instance per stream.
///
/// Every field is initialised to the reference decoder's reset values, so a
/// fresh [`Mr122Decoder`] and a fresh reference decoder agree from the first
/// frame onward. Playing a track twice therefore yields identical samples.
#[derive(Debug, Clone)]
pub struct Mr122Decoder {
    /// Excitation history followed by the current subframe.
    old_exc: [i16; EXC_LEN],
    /// Synthesis filter memory.
    mem_syn: [i16; M],
    /// Previous subframe's integer pitch lag.
    old_t0: i16,
    /// Previous frame's quantised line spectral pairs.
    lsp_old: [i16; M],
    /// Previous frame's quantised line spectral frequencies.
    past_lsf_q: [i16; M],
    /// The LSF predictor's residual memory.
    past_r_q: [i16; M],
    /// The code-gain predictor's quantised-energy history.
    past_qua_en: [i16; 4],
    // Post filter.
    mem_syn_pst: [i16; M],
    res2: [i16; L_SUBFR],
    synth_buf: [i16; FRAME_SAMPLES + M],
    agc_past_gain: i16,
    preemph_mem: i16,
    // Output high-pass filter.
    hp_y2_hi: i16,
    hp_y2_lo: i16,
    hp_y1_hi: i16,
    hp_y1_lo: i16,
    hp_x0: i16,
    hp_x1: i16,
}

impl Default for Mr122Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Mr122Decoder {
    /// A decoder in the reference's reset state.
    pub fn new() -> Self {
        Mr122Decoder {
            old_exc: [0; EXC_LEN],
            mem_syn: [0; M],
            old_t0: 40,
            lsp_old: [
                30000, 26000, 21000, 15000, 8000, 0, -8000, -15000, -21000, -26000,
            ],
            past_lsf_q: {
                let mut lsf = [0i16; M];
                lsf.copy_from_slice(&tables::MEAN_LSF_5);
                lsf
            },
            past_r_q: [0; M],
            past_qua_en: [MIN_ENERGY_MR122; 4],
            mem_syn_pst: [0; M],
            res2: [0; L_SUBFR],
            synth_buf: [0; FRAME_SAMPLES + M],
            agc_past_gain: 4096,
            preemph_mem: 0,
            hp_y2_hi: 0,
            hp_y2_lo: 0,
            hp_y1_hi: 0,
            hp_y1_lo: 0,
            hp_x0: 0,
            hp_x1: 0,
        }
    }

    /// Decodes one 31-octet MR122 payload into 160 samples.
    ///
    /// The payload length is checked by the caller; anything else here is
    /// total, so no input can panic.
    pub fn decode_frame(&mut self, payload: &[u8; 31]) -> [i16; FRAME_SAMPLES] {
        let parameters = unpack(payload);
        let mut synth = [0i16; FRAME_SAMPLES];
        let mut az = [[0i16; MP1]; SUBFRAMES];

        let mut cursor = 0usize;
        let (lsp_mid, lsp_new) = self.decode_lsf(&parameters[cursor..cursor + 5]);
        cursor += 5;
        interpolate_lpc(&self.lsp_old, &lsp_mid, &lsp_new, &mut az);
        self.lsp_old = lsp_new;

        let mut t0 = self.old_t0;
        for (subframe, az_subframe) in az.iter().enumerate() {
            let i_subfr = subframe * L_SUBFR;
            // Subframes 1 and 3 carry an absolute lag; 2 and 4 a relative one.
            let pit_flag = if subframe == 2 { 0 } else { i_subfr as i16 };

            let index = parameters[cursor];
            cursor += 1;
            let mut t0_frac;
            (t0, t0_frac) = dec_lag6(index, pit_flag, t0);
            // A relative index of 61 or more is outside the range the
            // encoder can produce, so the reference distrusts it and repeats
            // the previous subframe's lag. `t0` itself is overwritten, which
            // matters: the next subframe decodes relative to it.
            if pit_flag != 0 && index >= 61 {
                t0 = self.old_t0;
                t0_frac = 0;
            }
            self.predict_long_term(t0, t0_frac);

            let index = parameters[cursor];
            cursor += 1;
            // MR122 quantises the pitch gain to 14 bits of the table entry.
            let gain_pit = tables::QUA_GAIN_PITCH[(index as usize) & 15] & !3;

            let mut code = decode_10i40_35bits(&parameters[cursor..cursor + 10]);
            cursor += 10;

            // Pitch sharpening at twice the pitch gain, saturated. The
            // reference open-codes this shift here and calls `shl` for the
            // same value a few lines below; the two are identical for every
            // 16-bit input, so it is spelled once.
            let pit_sharp = shl(gain_pit, 1);
            let lag = t0 as usize;
            for i in lag..L_SUBFR {
                let temp = mult(code[i - lag], pit_sharp);
                code[i] = add(code[i], temp);
            }

            let index = parameters[cursor];
            cursor += 1;
            let gain_code = self.decode_code_gain(index, &code);

            // The reference latches `gain_pit` into a pitch-sharpening carry
            // here, clamped to `SHARPMAX`. Only the modes below MR102 read
            // that carry back on the following subframe, so for MR122 it is
            // write-only and is not kept as state at all.
            let pit_sharp = shl(gain_pit, 1);

            // The reference builds a second, sharpened excitation whenever
            // the doubled pitch gain passes 0.5, and blends it in below.
            let mut excp = [0i16; L_SUBFR];
            let sharpened = pit_sharp > 16384;
            if sharpened {
                for (i, slot) in excp.iter_mut().enumerate() {
                    let temp = mult(self.exc()[i], pit_sharp);
                    let product = l_mult(temp, gain_pit) >> 1;
                    *slot = round16(product);
                }
            }

            // MR122 halves the pitch factor and compensates with one more
            // shift, keeping the excitation in range at 12.2 kbit/s.
            let pitch_fac = gain_pit >> 1;
            let tmp_shift = 2i16;

            let mut exc_enhanced = [0i16; L_SUBFR];
            for i in 0..L_SUBFR {
                exc_enhanced[i] = self.exc()[i];
                let mut accumulator = l_mult(self.exc()[i], pitch_fac);
                accumulator = l_mac(accumulator, code[i], gain_code);
                accumulator = l_shl(accumulator, tmp_shift);
                self.exc_mut()[i] = round16(accumulator);
            }

            // Phase dispersion's closing mix. Its dispersion proper is
            // skipped for MR122; this loop is not, and it is deliberately not
            // `l_mac`: the reference shifts the product without `L_mult`'s
            // single special case.
            for (i, sample) in exc_enhanced.iter_mut().enumerate() {
                let mut accumulator = l_mult(*sample, pitch_fac);
                let contribution = (i32::from(code[i]) * i32::from(gain_code)) << 1;
                accumulator = l_add(accumulator, contribution);
                accumulator = l_shl(accumulator, tmp_shift);
                *sample = round16(accumulator);
            }

            let target = &mut synth[i_subfr..i_subfr + L_SUBFR];
            if sharpened {
                for (slot, enhanced) in excp.iter_mut().zip(exc_enhanced.iter()) {
                    *slot = add(*slot, *enhanced);
                }
                agc2(&exc_enhanced, &mut excp);
                syn_filt(az_subframe, &excp, target, &mut self.mem_syn, false);
            } else {
                syn_filt(az_subframe, &exc_enhanced, target, &mut self.mem_syn, false);
            }
            self.mem_syn.copy_from_slice(&target[L_SUBFR - M..]);

            self.old_exc.copy_within(L_SUBFR.., 0);
            self.old_t0 = t0;
        }

        self.post_filter(&mut synth, &az);
        self.post_process(&mut synth);
        // The reference masks its output to 13 bits, matching the handset
        // DACs the codec was written for.
        for sample in synth.iter_mut() {
            *sample &= !7;
        }
        synth
    }

    /// The current subframe's excitation window inside the history buffer.
    fn exc(&self) -> &[i16] {
        &self.old_exc[EXC_HISTORY..]
    }

    fn exc_mut(&mut self) -> &mut [i16] {
        &mut self.old_exc[EXC_HISTORY..]
    }

    /// `D_plsf_5`: split matrix quantiser with first-order MA prediction.
    fn decode_lsf(&mut self, indices: &[i16]) -> ([i16; M], [i16; M]) {
        let mut lsf1_r = [0i16; M];
        let mut lsf2_r = [0i16; M];

        let take = |table: &[i16], index: i16, span: usize| -> [i16; 4] {
            let base = ((index as usize) & (span - 1)) * 4;
            [
                table[base],
                table[base + 1],
                table[base + 2],
                table[base + 3],
            ]
        };

        let first = take(&tables::DICO1_LSF_5, indices[0], 128);
        lsf1_r[0] = first[0];
        lsf1_r[1] = first[1];
        lsf2_r[0] = first[2];
        lsf2_r[1] = first[3];

        let second = take(&tables::DICO2_LSF_5, indices[1], 256);
        lsf1_r[2] = second[0];
        lsf1_r[3] = second[1];
        lsf2_r[2] = second[2];
        lsf2_r[3] = second[3];

        // The third subvector carries its sign in the low bit of the index.
        let sign = indices[2] & 1;
        let third = take(&tables::DICO3_LSF_5, indices[2] >> 1, 256);
        if sign == 0 {
            lsf1_r[4] = third[0];
            lsf1_r[5] = third[1];
            lsf2_r[4] = third[2];
            lsf2_r[5] = third[3];
        } else {
            lsf1_r[4] = negate(third[0]);
            lsf1_r[5] = negate(third[1]);
            lsf2_r[4] = negate(third[2]);
            lsf2_r[5] = negate(third[3]);
        }

        let fourth = take(&tables::DICO4_LSF_5, indices[3], 256);
        lsf1_r[6] = fourth[0];
        lsf1_r[7] = fourth[1];
        lsf2_r[6] = fourth[2];
        lsf2_r[7] = fourth[3];

        let fifth = take(&tables::DICO5_LSF_5, indices[4], 64);
        lsf1_r[8] = fifth[0];
        lsf1_r[9] = fifth[1];
        lsf2_r[8] = fifth[2];
        lsf2_r[9] = fifth[3];

        let mut lsf1_q = [0i16; M];
        let mut lsf2_q = [0i16; M];
        for i in 0..M {
            let predicted = mult(self.past_r_q[i], LSP_PRED_FAC_MR122);
            let predicted = add(tables::MEAN_LSF_5[i], predicted);
            lsf1_q[i] = add(lsf1_r[i], predicted);
            lsf2_q[i] = add(lsf2_r[i], predicted);
            self.past_r_q[i] = lsf2_r[i];
        }

        reorder_lsf(&mut lsf1_q);
        reorder_lsf(&mut lsf2_q);
        self.past_lsf_q = lsf2_q;
        (lsf_to_lsp(&lsf1_q), lsf_to_lsp(&lsf2_q))
    }

    /// `Pred_lt_3or6` at 1/6 resolution: the adaptive-codebook contribution.
    ///
    /// The filtered history is written back into the excitation buffer two
    /// samples at a time, and a lag shorter than the subframe deliberately
    /// reads what this same loop has already written — that periodic
    /// extension is how the adaptive codebook reproduces pitch pulses closer
    /// together than one subframe.
    fn predict_long_term(&mut self, t0: i16, t0_frac: i16) {
        // Every lag the bitstream can encode lies inside the history buffer:
        // subframes 1 and 3 yield 17..=143 and subframes 2 and 4 yield
        // 17..=144. The clamp restates that as a property of the code, so no
        // index below can leave the array whatever the input bytes say.
        let lag = t0.clamp(17, 144) as usize;
        let mut frac = -t0_frac;
        let mut base = EXC_HISTORY as isize - lag as isize;
        if frac < 0 {
            frac += 6;
            base -= 1;
        }
        let frac = frac.clamp(0, 6) as usize;

        // The 1/6-resolution filter, deinterleaved into the phase pair the
        // convolution below walks.
        let mut coefficients = [0i16; 2 * (L_INTERPOL - 1)];
        let mut k = 0usize;
        for quad in coefficients.chunks_exact_mut(4) {
            quad[0] = tables::INTER_6[frac + k];
            quad[1] = tables::INTER_6[6 - frac + k];
            k += 6;
            quad[2] = tables::INTER_6[frac + k];
            quad[3] = tables::INTER_6[6 - frac + k];
            k += 6;
        }

        let mut x0 = base;
        let mut written = EXC_HISTORY;
        for _ in 0..L_SUBFR / 2 {
            x0 += 1;
            let mut x2 = x0;
            let mut x3 = x0;
            x0 += 1;
            let mut c = 0usize;
            let mut s1 = 0x0000_4000i32;
            let mut s2 = 0x0000_4000i32;
            for _ in 0..(L_INTERPOL - 1) / 2 {
                s2 = mac(s2, self.old_exc[x3 as usize], coefficients[c]);
                x3 -= 1;
                s1 = mac(s1, self.old_exc[x3 as usize], coefficients[c]);
                c += 1;
                s1 = mac(s1, self.old_exc[x2 as usize], coefficients[c]);
                x2 += 1;
                s2 = mac(s2, self.old_exc[x2 as usize], coefficients[c]);
                c += 1;
                s2 = mac(s2, self.old_exc[x3 as usize], coefficients[c]);
                x3 -= 1;
                s1 = mac(s1, self.old_exc[x3 as usize], coefficients[c]);
                c += 1;
                s1 = mac(s1, self.old_exc[x2 as usize], coefficients[c]);
                x2 += 1;
                s2 = mac(s2, self.old_exc[x2 as usize], coefficients[c]);
                c += 1;
            }
            self.old_exc[written] = (s1 >> 15) as i16;
            self.old_exc[written + 1] = (s2 >> 15) as i16;
            written += 2;
        }
    }

    /// `d_gain_code` with `gc_pred`: the predicted fixed-codebook gain.
    fn decode_code_gain(&mut self, index: i16, code: &[i16; L_SUBFR]) -> i16 {
        let mut energy_code: i32 = 0;
        for &sample in code.iter() {
            energy_code = energy_code.wrapping_add((i32::from(sample) * i32::from(sample)) >> 3);
        }
        energy_code = energy_code.wrapping_shl(4);
        if energy_code >> 31 != 0 {
            energy_code = i32::MAX;
        }

        // 26214 is 0.8 in Q15: the reference's energy normalisation.
        energy_code = (i32::from(round16(energy_code)) * 26214) << 1;
        let (exp, frac) = log2(energy_code);
        let energy_code = (i32::from(exp - 30) << 16) + (i32::from(frac) << 1);

        let mut energy = MEAN_ENER_MR122;
        for i in 0..4 {
            let term = (i32::from(self.past_qua_en[i]) * i32::from(tables::PRED_MR122[i])) << 1;
            energy = l_add(energy, term);
        }
        let difference = l_sub(energy, energy_code);
        let exp_gcode0 = (difference >> 17) as i16;
        let frac_gcode0 = ((difference >> 2) - (i32::from(exp_gcode0) << 15)) as i16;

        let entry = ((index as usize) & 31) * 3;
        let gcode0 = shl(pow2(exp_gcode0, frac_gcode0) as i16, 4);
        let gain_code = shl(mult(gcode0, tables::QUA_GAIN_CODE[entry]), 1);

        // Only the MR122 quantised energy is kept; the other predictor is
        // read by modes this decoder does not accept.
        let quantised_energy = tables::QUA_GAIN_CODE[entry + 1];
        self.past_qua_en[3] = self.past_qua_en[2];
        self.past_qua_en[2] = self.past_qua_en[1];
        self.past_qua_en[1] = self.past_qua_en[0];
        self.past_qua_en[0] = quantised_energy;
        gain_code
    }

    /// `Post_Filter`: formant emphasis, tilt compensation, and gain control.
    fn post_filter(&mut self, synth: &mut [i16; FRAME_SAMPLES], az: &[[i16; MP1]; SUBFRAMES]) {
        self.synth_buf[M..].copy_from_slice(synth);
        for (subframe, az_subframe) in az.iter().enumerate() {
            let i_subfr = subframe * L_SUBFR;
            let ap3 = weight_ai(az_subframe, &GAMMA3_MR122);
            let ap4 = weight_ai(az_subframe, &GAMMA4_MR122);

            let window = &self.synth_buf[i_subfr..i_subfr + M + L_SUBFR];
            residu(&ap3, window, &mut self.res2);

            // The impulse response of the two weighted filters in cascade.
            let mut impulse = [0i16; L_H];
            impulse[..MP1].copy_from_slice(&ap3);
            let source = impulse;
            let mut memory = [0i16; M];
            syn_filt(&ap4, &source, &mut impulse, &mut memory, false);

            let mut energy: i32 = 0;
            for &tap in impulse.iter().rev() {
                match double_product(tap, tap) {
                    // The reference abandons the sum on this one product
                    // instead of accumulating the saturated value.
                    None => {
                        energy = i32::MAX;
                        break;
                    }
                    Some(term) => energy = l_add(energy, term),
                }
            }
            let temp1 = (energy >> 16) as i16;
            let mut energy: i32 = 0;
            for i in (0..L_H - 1).rev() {
                match double_product(impulse[i], impulse[i + 1]) {
                    None => {
                        energy = i32::MAX;
                        break;
                    }
                    Some(term) => energy = l_add(energy, term),
                }
            }
            let mut temp2 = (energy >> 16) as i16;
            if temp2 <= 0 {
                temp2 = 0;
            } else {
                let mut scaled = (i32::from(temp2) * i32::from(MU)) >> 15;
                // The reference sign-extends bit 16 before narrowing.
                scaled |= -(scaled & 0x0001_0000);
                temp2 = div_s(scaled as i16, temp1);
            }

            self.preemphasis(temp2);
            let target = &mut synth[i_subfr..i_subfr + L_SUBFR];
            let residual = self.res2;
            syn_filt(&ap4, &residual, target, &mut self.mem_syn_pst, true);
            let reference = &self.synth_buf[M + i_subfr..M + i_subfr + L_SUBFR];
            self.agc_past_gain = agc(self.agc_past_gain, reference, target);
        }
        let tail = self.synth_buf.len() - M;
        self.synth_buf.copy_within(tail.., 0);
    }

    /// `preemphasis`: the tilt-compensation filter over one subframe.
    fn preemphasis(&mut self, g: i16) {
        let carry = self.res2[L_SUBFR - 1];
        for i in (1..L_SUBFR).rev() {
            let scaled = mult(g, self.res2[i - 1]);
            self.res2[i] = sub(self.res2[i], scaled);
        }
        let scaled = mult(g, self.preemph_mem);
        self.res2[0] = sub(self.res2[0], scaled);
        self.preemph_mem = carry;
    }

    /// `Post_Process`: the 80 Hz output high-pass filter and upscaling.
    fn post_process(&mut self, signal: &mut [i16; FRAME_SAMPLES]) {
        for sample in signal.iter_mut() {
            let x2 = self.hp_x1;
            self.hp_x1 = self.hp_x0;
            self.hp_x0 = *sample;
            let mut accumulator = i32::from(self.hp_y1_hi) * i32::from(HP_A[1]);
            accumulator =
                accumulator.wrapping_add((i32::from(self.hp_y1_lo) * i32::from(HP_A[1])) >> 15);
            accumulator = accumulator.wrapping_add(i32::from(self.hp_y2_hi) * i32::from(HP_A[2]));
            accumulator =
                accumulator.wrapping_add((i32::from(self.hp_y2_lo) * i32::from(HP_A[2])) >> 15);
            accumulator = accumulator.wrapping_add(i32::from(self.hp_x0) * i32::from(HP_B[0]));
            accumulator = accumulator.wrapping_add(i32::from(self.hp_x1) * i32::from(HP_B[1]));
            accumulator = accumulator.wrapping_add(i32::from(x2) * i32::from(HP_B[2]));
            accumulator = l_shl(accumulator, 3);
            *sample = round16(l_shl(accumulator, 1));
            self.hp_y2_hi = self.hp_y1_hi;
            self.hp_y2_lo = self.hp_y1_lo;
            self.hp_y1_hi = (accumulator >> 16) as i16;
            self.hp_y1_lo = ((accumulator >> 1) - (i32::from(self.hp_y1_hi) << 15)) as i16;
        }
    }
}

/// A plain wrapping multiply-accumulate, as the reference's `Syn_filt` and
/// `Pred_lt` use: no saturation, unlike `L_mac`.
fn mac(accumulator: i32, var1: i16, var2: i16) -> i32 {
    accumulator.wrapping_add(i32::from(var1) * i32::from(var2))
}

/// `x * y << 1` as the post filter's energy sums spell it out inline.
///
/// Returns `None` for the one product the reference special-cases,
/// `0x4000_0000`, because there it abandons the running sum rather than
/// adding a saturated term.
fn double_product(var1: i16, var2: i16) -> Option<i32> {
    let product = i32::from(var1) * i32::from(var2);
    if product != 0x4000_0000 {
        Some(product << 1)
    } else {
        None
    }
}

/// Unpacks a storage payload into the 57 MR122 parameters.
///
/// The storage format carries the frame's bits sorted by subjective
/// importance; `REORDER_MR122` puts them back into parameter order before
/// they are read off as fields.
fn unpack(payload: &[u8; 31]) -> [i16; 57] {
    let mut bits = [0i16; 244];
    for (i, &position) in tables::REORDER_MR122.iter().enumerate() {
        let bit = (payload[i >> 3] >> ((!i) & 7)) & 1;
        bits[position as usize] = i16::from(bit);
    }
    let mut parameters = [0i16; 57];
    let mut offset = 0usize;
    for (parameter, &width) in parameters.iter_mut().zip(tables::BITNO_MR122.iter()) {
        let mut value = 0i16;
        for _ in 0..width {
            value = (value << 1) | bits[offset];
            offset += 1;
        }
        *parameter = value;
    }
    parameters
}

/// `Dec_lag6`: the pitch lag at 1/6 sample resolution.
fn dec_lag6(index: i16, i_subfr: i16, previous_t0: i16) -> (i16, i16) {
    if i_subfr == 0 {
        if index < 463 {
            let mut i = (i32::from(index + 5) * 5462) >> 15;
            i += 17;
            let t0 = i as i16;
            let sixfold = (((i << 1) + i) << 1) as i16;
            (t0, index - sixfold + 105)
        } else {
            (index - 368, 0)
        }
    } else {
        let mut t0_min = previous_t0 - 5;
        if t0_min < PIT_MIN_MR122 {
            t0_min = PIT_MIN_MR122;
        }
        let mut t0_max = t0_min + 9;
        if t0_max > PIT_MAX {
            t0_max = PIT_MAX;
            t0_min = t0_max - 9;
        }
        let i = ((i32::from(index + 5) * 5462) >> 15) as i16 - 1;
        let t0 = i + t0_min;
        let sixfold = (i + (i << 1)) << 1;
        (t0, index - 3 - sixfold)
    }
}

/// `dec_10i40_35bits`: ten signed pulses over five interleaved tracks.
fn decode_10i40_35bits(indices: &[i16]) -> [i16; L_SUBFR] {
    let mut code = [0i16; L_SUBFR];
    for track in 0..5usize {
        let packed = indices[track];
        let position = tables::DGRAY[(packed & 7) as usize] as usize * 5 + track;
        let mut sign: i16 = if (packed >> 3) & 1 == 0 { 4096 } else { -4096 };
        code[position] = sign;

        let second = tables::DGRAY[(indices[track + 5] & 7) as usize] as usize * 5 + track;
        if second < position {
            sign = negate(sign);
        }
        code[second] = code[second].wrapping_add(sign);
    }
    code
}

/// `Reorder_lsf`: enforces the minimum spacing between line spectral
/// frequencies.
fn reorder_lsf(lsf: &mut [i16; M]) {
    let mut minimum = LSF_GAP;
    for value in lsf.iter_mut() {
        if *value < minimum {
            *value = minimum;
            minimum += LSF_GAP;
        } else {
            minimum = *value + LSF_GAP;
        }
    }
}

/// `Lsf_lsp`: frequency to cosine domain through the tabulated cosine.
fn lsf_to_lsp(lsf: &[i16; M]) -> [i16; M] {
    let mut lsp = [0i16; M];
    for (slot, &frequency) in lsp.iter_mut().zip(lsf.iter()) {
        // `Reorder_lsf` guarantees an increasing, positive sequence, and the
        // quantiser cannot reach the top of the table; the clamp makes that a
        // property of the code so a hostile payload cannot index out of it.
        let index = (frequency >> 8).clamp(0, 63) as usize;
        let offset = i32::from(frequency & 0x00ff);
        let table = &tables::LSF_LSP_TABLE;
        let step = i32::from(table[index + 1] - table[index]);
        *slot = table[index] + ((step * offset) >> 8) as i16;
    }
    lsp
}

/// `Int_lpc_1and3`: two transmitted LSP sets become four subframe filters.
fn interpolate_lpc(
    lsp_old: &[i16; M],
    lsp_mid: &[i16; M],
    lsp_new: &[i16; M],
    az: &mut [[i16; MP1]; SUBFRAMES],
) {
    let mut blended = [0i16; M];
    for i in 0..M {
        blended[i] = (lsp_old[i] >> 1) + (lsp_mid[i] >> 1);
    }
    az[0] = lsp_to_az(&blended);
    az[1] = lsp_to_az(lsp_mid);
    for i in 0..M {
        blended[i] = (lsp_mid[i] >> 1) + (lsp_new[i] >> 1);
    }
    az[2] = lsp_to_az(&blended);
    az[3] = lsp_to_az(lsp_new);
}

/// `Get_lsp_pol`: the symmetric/antisymmetric polynomial of five LSPs.
fn lsp_polynomial(lsp: &[i16], f: &mut [i32; 6]) {
    f[0] = 0x0100_0000;
    f[1] = -(i32::from(lsp[0])) << 10;
    let mut source = 2usize;
    for i in 2..=5usize {
        f[i] = f[i - 2];
        let mut slot = i;
        for _ in 1..i {
            let hi = (f[slot - 1] >> 16) as i16;
            let lo = ((f[slot - 1] >> 1) - (i32::from(hi) << 15)) as i16;
            let mut term = i32::from(hi) * i32::from(lsp[source]);
            term = term.wrapping_add((i32::from(lo) * i32::from(lsp[source])) >> 15);
            f[slot] = f[slot].wrapping_add(f[slot - 2]);
            f[slot] = f[slot].wrapping_sub(term << 2);
            slot -= 1;
        }
        f[slot] = f[slot].wrapping_sub(i32::from(lsp[source]) << 10);
        source += 2;
    }
}

/// `Lsp_Az`: line spectral pairs to direct-form predictor coefficients.
fn lsp_to_az(lsp: &[i16; M]) -> [i16; MP1] {
    let mut f1 = [0i32; 6];
    let mut f2 = [0i32; 6];
    lsp_polynomial(&lsp[0..], &mut f1);
    lsp_polynomial(&lsp[1..], &mut f2);
    for i in (1..=5usize).rev() {
        f1[i] = f1[i].wrapping_add(f1[i - 1]);
        f2[i] = f2[i].wrapping_sub(f2[i - 1]);
    }
    let mut a = [0i16; MP1];
    a[0] = 4096;
    for i in 1..=5usize {
        let sum = f1[i].wrapping_add(f2[i]).wrapping_add(1 << 12);
        let difference = f1[i].wrapping_sub(f2[i]).wrapping_add(1 << 12);
        a[i] = (sum >> 13) as i16;
        a[11 - i] = (difference >> 13) as i16;
    }
    a
}

/// `Weight_Ai`: bandwidth expansion of a predictor by a fixed factor set.
fn weight_ai(a: &[i16; MP1], factors: &[i16; M]) -> [i16; MP1] {
    let mut weighted = [0i16; MP1];
    weighted[0] = a[0];
    for i in 1..=M {
        weighted[i] = ((i32::from(a[i]) * i32::from(factors[i - 1]) + 0x4000) >> 15) as i16;
    }
    weighted
}

/// `Residu`: the LP residual of a windowed signal.
///
/// `input` must carry `M` samples of history ahead of the `L_SUBFR` samples
/// to filter.
fn residu(coefficients: &[i16; MP1], input: &[i16], output: &mut [i16; L_SUBFR]) {
    for (i, slot) in output.iter_mut().enumerate() {
        let mut accumulator = 0x0000_0800i32;
        for j in 0..=M {
            accumulator = mac(accumulator, coefficients[j], input[M + i - j]);
        }
        *slot = (accumulator >> 12) as i16;
    }
}

/// The reference's synthesis-filter output clamp, edge cases included.
///
/// Note that an accumulator of exactly `0x07ff_ffff` falls through to
/// `i16::MIN` rather than `i16::MAX`: the first test excludes it and the
/// second is a strict inequality. That asymmetry is in the reference and is
/// reproduced deliberately.
fn syn_saturate(accumulator: i32) -> i16 {
    if (accumulator.wrapping_add(134_217_728) as u32) < 0x0fff_ffff {
        (accumulator >> 12) as i16
    } else if accumulator > 0x07ff_ffff {
        i16::MAX
    } else {
        i16::MIN
    }
}

/// `Syn_filt`: the all-pole synthesis filter.
fn syn_filt(a: &[i16; MP1], x: &[i16], y: &mut [i16], mem: &mut [i16; M], update: bool) {
    let length = y.len().min(x.len());
    let mut history = [0i16; M + L_SUBFR];
    history[..M].copy_from_slice(mem);
    for i in 0..length {
        let mut accumulator = mac(0x0000_0800, x[i], a[0]);
        for j in 1..=M {
            accumulator = accumulator.wrapping_sub(i32::from(a[j]) * i32::from(history[M + i - j]));
        }
        history[M + i] = syn_saturate(accumulator);
    }
    y[..length].copy_from_slice(&history[M..M + length]);
    if update {
        mem.copy_from_slice(&history[length..length + M]);
    }
}

/// `energy_new`: the subframe energy, with the reference's fallback for the
/// saturating case.
fn energy_new(signal: &[i16]) -> i32 {
    let mut sum = 0i32;
    for &sample in signal.iter() {
        sum = l_mac(sum, sample, sample);
    }
    if sum != i32::MAX {
        return sum >> 4;
    }
    let mut sum = 0i32;
    for &sample in signal.iter() {
        let scaled = sample >> 2;
        sum = l_mac(sum, scaled, scaled);
    }
    sum
}

/// `agc`: matches the filtered subframe's energy to the unfiltered one.
fn agc(past_gain: i16, reference: &[i16], target: &mut [i16]) -> i16 {
    let energy = energy_new(target);
    if energy == 0 {
        return 0;
    }
    let mut exp = norm_l(energy) - 1;
    let gain_out = round16(l_shl(energy, exp));

    let energy = energy_new(reference);
    let g0 = if energy == 0 {
        0
    } else {
        let shift = norm_l(energy);
        let gain_in = round16(energy << shift);
        exp -= shift;
        let ratio = div_s(gain_out, gain_in);
        let scaled = l_shr(i32::from(ratio) << 7, exp);
        let root = inv_sqrt(scaled);
        let widened = root << 9;
        let rounded = ((widened + 0x0000_8000) >> 16) as i16;
        ((i32::from(rounded) * i32::from(32767 - AGC_FAC)) >> 15) as i16
    };

    let mut gain = past_gain;
    for sample in target.iter_mut() {
        gain = ((i32::from(gain) * i32::from(AGC_FAC)) >> 15) as i16;
        gain = gain.wrapping_add(g0);
        let scaled = (i32::from(*sample) * i32::from(gain)) << 1;
        *sample = (scaled >> 13) as i16;
    }
    gain
}

/// `agc2`: the memoryless variant used for the sharpened excitation.
fn agc2(reference: &[i16], target: &mut [i16]) {
    let energy = energy_new(target);
    if energy == 0 {
        return;
    }
    let mut exp = norm_l(energy) - 1;
    let gain_out = round16(l_shl(energy, exp));

    let energy = energy_new(reference);
    let g0 = if energy == 0 {
        0
    } else {
        let shift = norm_l(energy);
        let gain_in = round16(l_shl(energy, shift));
        exp -= shift;
        let ratio = i32::from(div_s(gain_out, gain_in));
        let widened = if ratio > 0x00ff_ffff {
            i32::MAX
        } else if ratio < -16_777_216 {
            i32::MIN
        } else {
            ratio << 7
        };
        let root = inv_sqrt(l_shr(widened, exp));
        let scaled = if root > 0x003f_ffff {
            i32::MAX
        } else if root < -4_194_304 {
            i32::MIN
        } else {
            root << 9
        };
        round16(scaled)
    };

    for sample in target.iter_mut().rev() {
        let product = l_mult(*sample, g0);
        *sample = if product > 0x0fff_ffff {
            i16::MAX
        } else if product < -268_435_456 {
            i16::MIN
        } else {
            (product >> 13) as i16
        };
    }
}
