pub fn read_expected<'a>(data: &'a hopper_native::return_data::ReturnData, program: &hopper_native::Address) -> Result<&'a u64, hopper_native::ProgramError> {
    data.as_type_from::<u64>(program)
}
