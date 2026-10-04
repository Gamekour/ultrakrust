//! Small SPIR-V rewrites applied before naga.
//!
//! `split_combined_samplers`: Unity compiles some shaders (sprites, legacy particles, additive) with
//! combined image-samplers (`OpTypeSampledImage` variables), which naga's SPIR-V reader does not
//! accept. Each such variable becomes an image variable at its original binding plus a sampler
//! variable at `binding + SAMPLER_BINDING_OFFSET`, and every load of it becomes
//! load image + load sampler + `OpSampledImage`. Shader math is unchanged.

pub const SAMPLER_BINDING_OFFSET: u32 = 16;

const OP_TYPE_SAMPLER: u32 = 26;
const OP_TYPE_SAMPLED_IMAGE: u32 = 27;
const OP_TYPE_POINTER: u32 = 32;
const OP_FUNCTION: u32 = 54;
const OP_VARIABLE: u32 = 59;
const OP_LOAD: u32 = 61;
const OP_DECORATE: u32 = 71;
const OP_SAMPLED_IMAGE: u32 = 86;
const SC_UNIFORM_CONSTANT: u32 = 0;
const DEC_BINDING: u32 = 33;
const DEC_DESCRIPTOR_SET: u32 = 34;

struct Inst {
    op: u32,
    ops: Vec<u32>,
}

fn parse(words: &[u32]) -> Option<Vec<Inst>> {
    let mut out = Vec::new();
    let mut i = 5;
    while i < words.len() {
        let (op, len) = (words[i] & 0xffff, (words[i] >> 16) as usize);
        if len == 0 || i + len > words.len() {
            return None;
        }
        out.push(Inst { op, ops: words[i + 1..i + len].to_vec() });
        i += len;
    }
    Some(out)
}

fn emit(header: &[u32], insts: &[Inst], bound: u32) -> Vec<u32> {
    let mut w = header.to_vec();
    w[3] = bound;
    for x in insts {
        w.push(((x.ops.len() as u32 + 1) << 16) | x.op);
        w.extend_from_slice(&x.ops);
    }
    w
}

/// Returns the rewritten module, or `None` if it has no combined image-samplers (or can't be parsed).
pub fn split_combined_samplers(words: &[u32]) -> Option<Vec<u32>> {
    if words.len() < 5 {
        return None;
    }
    let mut insts = parse(words)?;
    let mut bound = words[3];
    let mut fresh = || {
        bound += 1;
        bound - 1
    };
    // sampled image type -> image type
    let sampled: Vec<(u32, u32)> = insts.iter().filter(|x| x.op == OP_TYPE_SAMPLED_IMAGE).map(|x| (x.ops[0], x.ops[1])).collect();
    if sampled.is_empty() {
        return None;
    }
    // pointer types (UniformConstant) to sampled images
    let ptrs: Vec<(u32, u32)> = insts
        .iter()
        .filter(|x| x.op == OP_TYPE_POINTER && x.ops[1] == SC_UNIFORM_CONSTANT && sampled.iter().any(|s| s.0 == x.ops[2]))
        .map(|x| (x.ops[0], x.ops[2]))
        .collect();
    // combined variables: (var id, sampled image type)
    let vars: Vec<(u32, u32)> = insts
        .iter()
        .filter(|x| x.op == OP_VARIABLE && x.ops[2] == SC_UNIFORM_CONSTANT)
        .filter_map(|x| ptrs.iter().find(|p| p.0 == x.ops[0]).map(|p| (x.ops[1], p.1)))
        .collect();
    if vars.is_empty() {
        return None;
    }
    // one sampler type and its pointer
    let sampler_ty = match insts.iter().find(|x| x.op == OP_TYPE_SAMPLER) {
        Some(x) => x.ops[0],
        None => {
            let id = fresh();
            let at = insts.iter().position(|x| x.op == OP_TYPE_SAMPLED_IMAGE).unwrap();
            insts.insert(at, Inst { op: OP_TYPE_SAMPLER, ops: vec![id] });
            id
        }
    };
    let sampler_ptr = fresh();
    let after_types = insts.iter().position(|x| x.op == OP_TYPE_SAMPLED_IMAGE).unwrap() + 1;
    insts.insert(after_types, Inst { op: OP_TYPE_POINTER, ops: vec![sampler_ptr, SC_UNIFORM_CONSTANT, sampler_ty] });
    // pointer-to-image type per sampled image type
    let mut image_ptr = std::collections::HashMap::new();
    for &(si, img) in &sampled {
        let p = fresh();
        let at = insts.iter().position(|x| x.op == OP_TYPE_SAMPLED_IMAGE && x.ops[0] == si).unwrap() + 1;
        insts.insert(at, Inst { op: OP_TYPE_POINTER, ops: vec![p, SC_UNIFORM_CONSTANT, img] });
        image_ptr.insert(si, (p, img));
    }
    // retype the variables, add a sampler variable each (with decorations)
    let mut sampler_var = std::collections::HashMap::new();
    for &(v, si) in &vars {
        let (p_img, _) = image_ptr[&si];
        let at = insts.iter().position(|x| x.op == OP_VARIABLE && x.ops[1] == v).unwrap();
        insts[at].ops[0] = p_img;
        let sv = fresh();
        insts.insert(at + 1, Inst { op: OP_VARIABLE, ops: vec![sampler_ptr, sv, SC_UNIFORM_CONSTANT] });
        sampler_var.insert(v, sv);
        let set = insts.iter().find(|x| x.op == OP_DECORATE && x.ops[0] == v && x.ops[1] == DEC_DESCRIPTOR_SET).map(|x| x.ops[2]).unwrap_or(0);
        let binding = insts.iter().find(|x| x.op == OP_DECORATE && x.ops[0] == v && x.ops[1] == DEC_BINDING).map(|x| x.ops[2]).unwrap_or(0);
        let dec_at = insts.iter().position(|x| x.op == OP_DECORATE).unwrap_or(0);
        insts.insert(dec_at, Inst { op: OP_DECORATE, ops: vec![sv, DEC_BINDING, binding + SAMPLER_BINDING_OFFSET] });
        insts.insert(dec_at, Inst { op: OP_DECORATE, ops: vec![sv, DEC_DESCRIPTOR_SET, set] });
    }
    // rewrite loads inside functions
    let first_fn = insts.iter().position(|x| x.op == OP_FUNCTION).unwrap_or(insts.len());
    let mut i = first_fn;
    while i < insts.len() {
        if insts[i].op == OP_LOAD {
            let (ty, res, ptr) = (insts[i].ops[0], insts[i].ops[1], insts[i].ops[2]);
            if let Some(&sv) = sampler_var.get(&ptr) {
                let (_, img) = image_ptr[&ty];
                let (li, ls) = (fresh(), fresh());
                let extra: Vec<u32> = insts[i].ops[3..].to_vec();
                insts[i] = Inst { op: OP_LOAD, ops: [vec![img, li, ptr], extra].concat() };
                insts.insert(i + 1, Inst { op: OP_LOAD, ops: vec![sampler_ty, ls, sv] });
                insts.insert(i + 2, Inst { op: OP_SAMPLED_IMAGE, ops: vec![ty, res, li, ls] });
                i += 3;
                continue;
            }
        }
        i += 1;
    }
    Some(emit(&words[..5], &insts, bound))
}

/// Everything a Unity SPIR-V module needs before naga reads it.
pub fn prepare(words: &[u32]) -> Vec<u32> {
    split_combined_samplers(words).unwrap_or_else(|| words.to_vec())
}
