pub mod ton;
pub mod tycho;

pub struct KeyBlockInfo {
    pub seqno: u32,
    pub prev_seqno: u32,
    pub v_set: everscale_types::models::ValidatorSet,
    pub signatures: Vec<everscale_types::models::BlockSignature>,
}
