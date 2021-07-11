use {
    crate::{
        error::MetaplexError,
        state::{
            Key, WinningConfigItem, WinningConfigType, MAX_PRIZE_TRACKING_TICKET_SIZE, PREFIX,
        },
        utils::{
            assert_derivation, assert_is_ata, assert_owned_by, common_redeem_checks,
            common_redeem_finish, common_winning_config_checks, create_or_allocate_account_raw,
            get_amount_from_token_account, CommonRedeemCheckArgs, CommonRedeemFinishArgs,
            CommonRedeemReturn, CommonWinningConfigCheckReturn,
        },
    },
    arrayref::{array_mut_ref, array_ref, mut_array_refs},
    solana_program::{
        account_info::{next_account_info, AccountInfo},
        entrypoint::ProgramResult,
        program::invoke_signed,
        program_error::ProgramError,
        pubkey::Pubkey,
    },
    spl_token_metadata::{
        instruction::mint_edition_from_master_edition_via_vault_proxy,
        utils::get_supply_off_master_edition,
    },
    spl_token_vault::state::SafetyDepositBox,
};

fn count_item_amount_by_safety_deposit_order(
    items: &Vec<WinningConfigItem>,
    safety_deposit_index: u8,
) -> u64 {
    let item = items.iter().find_map(|i| {
        if i.safety_deposit_box_index == safety_deposit_index {
            Some(i)
        } else {
            None
        }
    });

    match item {
        Some(item) => item.amount as u64,
        None => 0u64,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn mint_edition<'a>(
    token_metadata_program_info: &AccountInfo<'a>,
    token_vault_program_info: &AccountInfo<'a>,
    new_metadata_account_info: &AccountInfo<'a>,
    new_edition_account_info: &AccountInfo<'a>,
    master_edition_account_info: &AccountInfo<'a>,
    edition_marker_info: &AccountInfo<'a>,
    mint_info: &AccountInfo<'a>,
    mint_authority_info: &AccountInfo<'a>,
    payer_info: &AccountInfo<'a>,
    auction_manager_info: &AccountInfo<'a>,
    safety_deposit_token_store_info: &AccountInfo<'a>,
    safety_deposit_box_info: &AccountInfo<'a>,
    vault_info: &AccountInfo<'a>,
    bidder_info: &AccountInfo<'a>,
    metadata_account_info: &AccountInfo<'a>,
    token_program_info: &AccountInfo<'a>,
    system_program_info: &AccountInfo<'a>,
    rent_info: &AccountInfo<'a>,
    actual_edition: u64,
    signer_seeds: &[&[u8]],
) -> ProgramResult {
    invoke_signed(
        &mint_edition_from_master_edition_via_vault_proxy(
            *token_metadata_program_info.key,
            *new_metadata_account_info.key,
            *new_edition_account_info.key,
            *master_edition_account_info.key,
            *mint_info.key,
            *edition_marker_info.key,
            *mint_authority_info.key,
            *payer_info.key,
            *auction_manager_info.key,
            *safety_deposit_token_store_info.key,
            *safety_deposit_box_info.key,
            *vault_info.key,
            *bidder_info.key,
            *metadata_account_info.key,
            *token_program_info.key,
            *token_vault_program_info.key,
            actual_edition,
        ),
        &[
            token_metadata_program_info.clone(),
            token_vault_program_info.clone(),
            new_metadata_account_info.clone(),
            new_edition_account_info.clone(),
            master_edition_account_info.clone(),
            edition_marker_info.clone(),
            mint_info.clone(),
            mint_authority_info.clone(),
            payer_info.clone(),
            auction_manager_info.clone(),
            safety_deposit_token_store_info.clone(),
            safety_deposit_box_info.clone(),
            vault_info.clone(),
            bidder_info.clone(),
            metadata_account_info.clone(),
            token_program_info.clone(),
            system_program_info.clone(),
            rent_info.clone(),
        ],
        &[&signer_seeds],
    )?;

    Ok(())
}

pub fn create_or_update_prize_tracking<'a>(
    program_id: &'a Pubkey,
    auction_manager_info: &AccountInfo<'a>,
    prize_tracking_ticket_info: &AccountInfo<'a>,
    metadata_account_info: &AccountInfo<'a>,
    payer_info: &AccountInfo<'a>,
    rent_info: &AccountInfo<'a>,
    system_info: &AccountInfo<'a>,
    master_edition_account_info: &AccountInfo<'a>,
    expected_redemptions: u64,
) -> Result<u64, ProgramError> {
    let metadata_data = metadata_account_info.data.borrow();
    let metadata_mint = Pubkey::new_from_array(*array_ref![metadata_data, 33, 32]);

    let bump = assert_derivation(
        program_id,
        prize_tracking_ticket_info,
        &[
            PREFIX.as_bytes(),
            program_id.as_ref(),
            auction_manager_info.key.as_ref(),
            metadata_mint.as_ref(),
        ],
    )?;

    let supply_snapshot: u64;
    if prize_tracking_ticket_info.data_is_empty() {
        create_or_allocate_account_raw(
            *program_id,
            prize_tracking_ticket_info,
            rent_info,
            system_info,
            payer_info,
            MAX_PRIZE_TRACKING_TICKET_SIZE,
            &[
                PREFIX.as_bytes(),
                program_id.as_ref(),
                auction_manager_info.key.as_ref(),
                metadata_mint.as_ref(),
                &[bump],
            ],
        )?;
        let data = &mut prize_tracking_ticket_info.data.borrow_mut();
        let output = array_mut_ref![data, 0, MAX_PRIZE_TRACKING_TICKET_SIZE];

        let (key, metadata, supply_snapshot_ptr, expected_redemptions_ptr, redemptions, _padding) =
            mut_array_refs![output, 1, 32, 8, 8, 8, 50];

        *key = [Key::PrizeTrackingTicketV1 as u8];
        metadata.copy_from_slice(metadata_account_info.key.as_ref());
        supply_snapshot = get_supply_off_master_edition(master_edition_account_info)?;
        *supply_snapshot_ptr = supply_snapshot.to_le_bytes();
        *redemptions = 1u64.to_le_bytes();
        *expected_redemptions_ptr = expected_redemptions.to_le_bytes();
    } else {
        // CPU is very precious in this large action, so we skip borsh's angry CPU usage.
        let data = &mut prize_tracking_ticket_info.data.borrow_mut();
        let output = array_mut_ref![data, 0, MAX_PRIZE_TRACKING_TICKET_SIZE];

        let (_key, _metadata, supply_snapshot_ptr, _expected_redemptions, redemptions, _padding) =
            mut_array_refs![output, 1, 32, 8, 8, 8, 50];
        supply_snapshot = u64::from_le_bytes(*supply_snapshot_ptr);
        let next_redemptions = u64::from_le_bytes(*redemptions)
            .checked_add(1)
            .ok_or(MetaplexError::NumericalOverflowError)?;
        *redemptions = next_redemptions.to_le_bytes();
    }

    Ok(supply_snapshot)
}

pub fn process_redeem_printing_v2_bid<'a>(
    program_id: &'a Pubkey,
    accounts: &'a [AccountInfo<'a>],
    edition_offset: u64,
    user_provided_win_index: u64,
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();

    let auction_manager_info = next_account_info(account_info_iter)?;
    let safety_deposit_token_store_info = next_account_info(account_info_iter)?;
    let new_edition_token_account_info = next_account_info(account_info_iter)?;
    let bid_redemption_info = next_account_info(account_info_iter)?;
    let safety_deposit_info = next_account_info(account_info_iter)?;
    let vault_info = next_account_info(account_info_iter)?;
    let _fraction_mint_info = next_account_info(account_info_iter)?;
    let auction_info = next_account_info(account_info_iter)?;
    let bidder_metadata_info = next_account_info(account_info_iter)?;
    let bidder_info = next_account_info(account_info_iter)?;
    let payer_info = next_account_info(account_info_iter)?;
    let token_program_info = next_account_info(account_info_iter)?;
    let token_vault_program_info = next_account_info(account_info_iter)?;
    let token_metadata_program_info = next_account_info(account_info_iter)?;
    let store_info = next_account_info(account_info_iter)?;
    let system_info = next_account_info(account_info_iter)?;
    let rent_info = next_account_info(account_info_iter)?;

    let prize_tracking_ticket_info = next_account_info(account_info_iter)?;
    let new_metadata_account_info = next_account_info(account_info_iter)?;
    let new_edition_account_info = next_account_info(account_info_iter)?;
    let master_edition_account_info = next_account_info(account_info_iter)?;
    let mint_info = next_account_info(account_info_iter)?;
    let edition_marker_info = next_account_info(account_info_iter)?;
    let mint_authority_info = next_account_info(account_info_iter)?;
    let metadata_account_info = next_account_info(account_info_iter)?;

    let new_edition_account_amount = get_amount_from_token_account(new_edition_token_account_info)?;

    assert_is_ata(
        new_edition_token_account_info,
        bidder_info.key,
        token_program_info.key,
        mint_info.key,
    )?;

    if new_edition_account_amount != 1 {
        return Err(MetaplexError::ProvidedAccountDoesNotContainOneToken.into());
    }

    let CommonRedeemReturn {
        auction_manager,
        redemption_bump_seed,
        cancelled,
        auction: _a,
        rent: _rent,
        win_index,
        token_metadata_program,
    } = common_redeem_checks(CommonRedeemCheckArgs {
        program_id,
        auction_manager_info,
        safety_deposit_token_store_info,
        destination_info: new_edition_token_account_info,
        bid_redemption_info,
        safety_deposit_info,
        vault_info,
        auction_info,
        bidder_metadata_info,
        bidder_info,
        token_program_info,
        token_vault_program_info,
        token_metadata_program_info,
        store_info,
        rent_info,
        is_participation: false,
        user_provided_win_index: Some(Some(user_provided_win_index as usize)),
        overwrite_win_index: None,
        assert_bidder_signer: false,
    })?;

    assert_owned_by(metadata_account_info, &token_metadata_program)?;

    let mut winning_item_index = None;
    if !cancelled {
        if let Some(winning_index) = win_index {
            if winning_index < auction_manager.settings.winning_configs.len() {
                let CommonWinningConfigCheckReturn {
                    winning_config_item,
                    winning_item_index: wii,
                } = common_winning_config_checks(
                    &auction_manager,
                    &safety_deposit_info,
                    winning_index,
                )?;

                winning_item_index = wii;

                if winning_config_item.winning_config_type != WinningConfigType::PrintingV2 {
                    return Err(MetaplexError::WrongBidEndpointForPrize.into());
                }
                let auction_manager_bump = assert_derivation(
                    program_id,
                    auction_manager_info,
                    &[PREFIX.as_bytes(), auction_info.key.as_ref()],
                )?;

                let safety_deposit_box_order = SafetyDepositBox::get_order(safety_deposit_info);

                let mut edition_offset_min: u64 = 1;
                let mut expected_redemptions: u64 = 0;

                // Given every single winning config item carries a u8, it is impossible to overflow
                // a u64 with the amount in it given the limited size. Avoid using checked add to save on cpu.
                for n in 0..auction_manager.settings.winning_configs.len() {
                    let matching = count_item_amount_by_safety_deposit_order(
                        &auction_manager.settings.winning_configs[n].items,
                        safety_deposit_box_order,
                    );

                    if n < winning_index {
                        edition_offset_min += matching
                    }
                    if prize_tracking_ticket_info.data_is_empty() {
                        expected_redemptions += matching
                    } else if n >= winning_index {
                        // no need to keep using this loop more than winning_index if we're not
                        // tabulating expected_redemptions
                        break;
                    }
                }

                let edition_offset_max = edition_offset_min
                    + count_item_amount_by_safety_deposit_order(
                        &auction_manager.settings.winning_configs[winning_index].items,
                        safety_deposit_box_order,
                    );

                if edition_offset < edition_offset_min || edition_offset >= edition_offset_max {
                    return Err(MetaplexError::InvalidEditionNumber.into());
                }

                let supply_snapshot = create_or_update_prize_tracking(
                    program_id,
                    auction_manager_info,
                    prize_tracking_ticket_info,
                    metadata_account_info,
                    payer_info,
                    rent_info,
                    system_info,
                    master_edition_account_info,
                    expected_redemptions,
                )?;

                let actual_edition = edition_offset
                    .checked_add(supply_snapshot)
                    .ok_or(MetaplexError::NumericalOverflowError)?;

                let signer_seeds = &[
                    PREFIX.as_bytes(),
                    auction_info.key.as_ref(),
                    &[auction_manager_bump],
                ];

                mint_edition(
                    token_metadata_program_info,
                    token_vault_program_info,
                    new_metadata_account_info,
                    new_edition_account_info,
                    master_edition_account_info,
                    edition_marker_info,
                    mint_info,
                    mint_authority_info,
                    payer_info,
                    auction_manager_info,
                    safety_deposit_token_store_info,
                    safety_deposit_info,
                    vault_info,
                    bidder_info,
                    metadata_account_info,
                    token_program_info,
                    system_info,
                    rent_info,
                    actual_edition,
                    signer_seeds,
                )?;
            }
        }
    };

    common_redeem_finish(CommonRedeemFinishArgs {
        program_id,
        auction_manager,
        auction_manager_info,
        bidder_metadata_info,
        rent_info,
        system_info,
        payer_info,
        bid_redemption_info,
        redemption_bump_seed,
        winning_index: win_index,
        bid_redeemed: true,
        participation_redeemed: false,
        winning_item_index,
        overwrite_win_index: None,
    })?;

    Ok(())
}
