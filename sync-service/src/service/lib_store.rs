use std::sync::OnceLock;

use tycho_types::models::{StateInit, StdAddr};
use tycho_types::prelude::*;

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
