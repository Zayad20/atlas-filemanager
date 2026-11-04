//! Microsoft Store add-on ownership. Read-only, so no window handle is needed;
//! an unpackaged build has no Store context and simply reports no entitlement.
use windows::Services::Store::StoreContext;

/// Developer-defined Product ID of the Premium add-on in Partner Center.
pub const PREMIUM_OFFER_TOKEN: &str = "atlas_premium";

/// Blocking: call from a worker thread, never from the UI thread.
pub fn owns_premium() -> bool {
    check().unwrap_or(false)
}

fn check() -> windows::core::Result<bool> {
    let license = StoreContext::GetDefault()?.GetAppLicenseAsync()?.get()?;
    for entry in license.AddOnLicenses()? {
        let add_on = entry.Value()?;
        if add_on.IsActive()? && add_on.InAppOfferToken()? == PREMIUM_OFFER_TOKEN {
            return Ok(true);
        }
    }
    Ok(false)
}
