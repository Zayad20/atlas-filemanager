use super::App;
use atlas_model::edition::{self, Feature};

impl App {
    /// All action entry points share the build's edition policy and feedback.
    pub(crate) fn allow_edition_action(&mut self, feature: Feature, count: usize) -> bool {
        let Err(limit) = edition::current().check(feature, count) else {
            return true;
        };
        let message = match feature {
            Feature::SavedConnections => {
                rust_i18n::t!("free_limit_connections", limit = limit.maximum)
            }
            Feature::Profiles => rust_i18n::t!("free_limit_profiles", limit = limit.maximum),
            Feature::BatchRename => rust_i18n::t!("free_limit_batch_rename", limit = limit.maximum),
            Feature::BatchConversion => {
                rust_i18n::t!("free_limit_batch_conversion", limit = limit.maximum)
            }
            Feature::Panes => rust_i18n::t!("free_limit_panes", limit = limit.maximum),
            Feature::ConversionsPerDay => {
                rust_i18n::t!("free_limit_daily_conversions", limit = limit.maximum)
            }
        };
        self.toast_info(message.into_owned());
        false
    }

    /// A conversion must fit both the per-operation and the per-day Free limits.
    pub(crate) fn allow_conversion_action(&mut self, count: usize) -> bool {
        self.allow_edition_action(Feature::BatchConversion, count)
            && self.allow_edition_action(
                Feature::ConversionsPerDay,
                atlas_core::usage::conversions_today() + count,
            )
    }

    /// Store build: the native purchase dialog for the add-on, owned by this
    /// window. Other builds cannot buy add-ons, so they open the Store listing
    /// to install the Store edition instead.
    pub(crate) fn buy_premium_in_store(&mut self) {
        const STORE_LISTING: &str = "ms-windows-store://pdp/?productid=9MX95D1GDMR3&cid=app";
        #[cfg(windows)]
        if let (true, Some(owner)) = (atlas_win_space::integration::is_packaged(), self.hwnd) {
            use atlas_core::license::store::Purchase;
            let tx = self.tx.clone();
            self.rt.spawn(async move {
                let result = match atlas_core::license::buy_in_store(owner).await {
                    Purchase::Owned => Ok(rust_i18n::t!("license_activated").into_owned()),
                    Purchase::Cancelled => return,
                    Purchase::Failed(why) => {
                        tracing::warn!(%why, "store purchase failed");
                        Err(rust_i18n::t!("license_store_failed", why = why).into_owned())
                    }
                };
                let _ = tx.send(atlas_model::app_state::BgResult::LicenseChanged(result));
            });
            return;
        }
        let _ = open::that(STORE_LISTING);
    }
}

/// User-facing, localized text for a license failure (the core error is English-only).
pub(super) fn license_error_text(error: &atlas_core::license::polar::Error) -> String {
    use atlas_core::license::polar::Error;
    match error {
        Error::NotConfigured => rust_i18n::t!("license_err_not_configured").into_owned(),
        Error::Invalid => rust_i18n::t!("license_err_invalid").into_owned(),
        Error::ActivationLimit => rust_i18n::t!("license_err_limit").into_owned(),
        Error::Network(why) => rust_i18n::t!("license_err_network", why = why).into_owned(),
    }
}
