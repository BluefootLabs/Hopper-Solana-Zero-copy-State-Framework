use quasar_lang::prelude::*;

#[event(discriminator = [100])]
pub struct HeapTestEvent {
    pub value: u64,
}
