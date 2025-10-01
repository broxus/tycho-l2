use anyhow::{Result, anyhow};
use num_bigint::BigInt;
use tycho_types::crc::crc_16;
use tycho_types::models::{Account, AccountState, BlockchainConfig};
use tycho_vm::{GasParams, RcStackValue, SafeRc, SmcInfoBase, VmStateBuilder};

pub trait AsGetterMethodId {
    fn as_getter_method_id(&self) -> u32;
}

impl<T: AsGetterMethodId + ?Sized> AsGetterMethodId for &T {
    fn as_getter_method_id(&self) -> u32 {
        T::as_getter_method_id(*self)
    }
}

impl<T: AsGetterMethodId + ?Sized> AsGetterMethodId for &mut T {
    fn as_getter_method_id(&self) -> u32 {
        T::as_getter_method_id(*self)
    }
}

impl AsGetterMethodId for u32 {
    fn as_getter_method_id(&self) -> u32 {
        *self
    }
}

impl AsGetterMethodId for str {
    fn as_getter_method_id(&self) -> u32 {
        let crc = crc_16(self.as_bytes());
        crc as u32 | 0x10000
    }
}

pub struct ExecutionContext<'a> {
    pub account: &'a Account,
    pub config: &'a BlockchainConfig,
}

impl ExecutionContext<'_> {
    pub fn call_getter(
        &self,
        method_id: impl AsGetterMethodId,
        args: Vec<RcStackValue>,
    ) -> Result<VmGetterOutput> {
        self.call_getter_impl(method_id.as_getter_method_id(), args)
    }

    fn call_getter_impl(
        &self,
        method_id: u32,
        mut args: Vec<RcStackValue>,
    ) -> Result<VmGetterOutput> {
        let state = match &self.account.state {
            AccountState::Active(state_init) => state_init,
            _ => anyhow::bail!("account is not active"),
        };
        let code = state.clone().code.ok_or(anyhow!("account has no code"))?;

        let block_lt = 0;
        let block_unixtime = tycho_util::time::now_sec();

        let smc = SmcInfoBase::new()
            .with_account_addr(self.account.address.clone())
            .with_account_balance(self.account.balance.clone())
            .with_config(self.config.params.clone())
            .with_block_lt(block_lt)
            .with_tx_lt(block_lt)
            .with_now(block_unixtime)
            .require_ton_v4()
            .require_ton_v6()
            .fill_unpacked_config()?
            .require_ton_v11();

        let data = state.clone().data.unwrap_or_default();

        args.push(SafeRc::new_dyn_value(BigInt::from(method_id)));

        let mut vm_state = VmStateBuilder::new()
            .with_code(code)
            .with_data(data)
            .with_stack(args)
            .with_smc_info(smc)
            .with_gas(GasParams::getter())
            .build();

        let exit_code = !vm_state.run();

        Ok(VmGetterOutput {
            exit_code,
            stack: vm_state.stack.items.clone(),
            success: exit_code == 0 || exit_code == 1,
        })
    }
}

pub struct VmGetterOutput {
    pub exit_code: i32,
    pub stack: Vec<RcStackValue>,
    pub success: bool,
}
