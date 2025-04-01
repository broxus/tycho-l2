use std::sync::OnceLock;

use everscale_types::error::Error;
use everscale_types::models::{StateInit, StdAddr, ValidatorSet};
use everscale_types::num::Tokens;
use everscale_types::prelude::*;

pub fn make_epoch_data(vset: &ValidatorSet) -> Result<Cell, Error> {
    let main_validator_count = vset.main.get() as usize;
    if vset.list.len() < main_validator_count {
        return Err(Error::InvalidData);
    }

    let mut total_weight = 0u64;
    let mut main_validators = Vec::new();
    for (i, item) in vset.list[..main_validator_count].iter().enumerate() {
        main_validators.push((i as u16, (item.public_key, item.weight)));
        total_weight = total_weight
            .checked_add(item.weight)
            .ok_or(Error::IntOverflow)?;
    }
    assert_eq!(main_validators.len(), main_validator_count);

    let Some(root) =
        Dict::<u16, (HashBytes, u64)>::try_from_sorted_slice(&main_validators)?.into_root()
    else {
        return Err(Error::CellUnderflow);
    };

    let cutoff_weight = (total_weight as u128) * 2 / 3 + 1;

    let mut b = CellBuilder::new();
    b.store_u32(vset.utime_since)?;
    b.store_u32(vset.utime_until)?;
    b.store_u16(vset.main.get())?;
    Tokens::new(cutoff_weight).store_into(&mut b, Cell::empty_context())?;
    b.store_reference(root)?;
    b.build()
}

pub fn make_state_init(owner: &StdAddr, id: u128) -> StateInit {
    StateInit {
        split_depth: None,
        special: None,
        code: Some(lib_store_code().clone()),
        data: Some(CellBuilder::build_from((owner, id)).unwrap()),
        libraries: Dict::new(),
    }
}

fn lib_store_code() -> &'static Cell {
    static CODE: OnceLock<Cell> = OnceLock::new();
    CODE.get_or_init(|| Boc::decode(include_bytes!("../../res/lib_store_code.boc")).unwrap())
}
