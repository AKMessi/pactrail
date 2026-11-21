//! Ledger presentation components; no engine or persistence authority.
mod blocks;
pub(crate) use blocks::{note, section};
pub(crate) mod glyphs;
pub(crate) mod input;
pub(crate) mod live;
mod panel;
pub(crate) use panel::{
    body_width as card_body_width, card, card_styled, field as card_field, group as card_group,
};
pub(crate) mod recovery;
