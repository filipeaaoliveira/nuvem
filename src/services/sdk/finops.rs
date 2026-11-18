//! Cost recommendations through the native client.
//!
//! Only the Recommender half moves. Cloud Billing budgets have no published
//! Rust client, so `load_budgets` stays on `gcloud billing budgets list` —
//! see `docs/decisions/0001-native-sdk-for-read-paths.md`.

use google_cloud_auth::credentials::Credentials;
use google_cloud_recommender_v1::client::Recommender;
use nuvem_domain::gcp::finops::{CostRecommendation, RecommendationState, RecommenderId};
use nuvem_domain::gcp::money::{MONTH_SECONDS, Money};

use crate::services::sdk::{ApiError, non_empty, timestamp_to_string};

pub struct SdkFinOpsService {
    recommender: Recommender,
}

impl SdkFinOpsService {
    pub async fn connect(credentials: Credentials) -> Result<Self, ApiError> {
        let recommender = Recommender::builder()
            .with_credentials(credentials)
            .build()
            .await
            .map_err(|error| ApiError::new(error.to_string()))?;
        Ok(Self { recommender })
    }

    /// The same service, built from an already-constructed client. See
    /// `SdkGkeService::from_clusters_for_test`.
    pub fn from_recommender_for_test(recommender: Recommender) -> Self {
        Self { recommender }
    }

    /// The same service, answering from the demo fixtures.
    ///
    /// Required by `sdk_client_cache!`: the macro's `Fixtures` arm calls this
    /// on every service in its list, so a fifteenth service cannot join that
    /// list without one. It takes no credentials, cannot fail and cannot
    /// block -- a client built by `from_stub` holds a stub where a
    /// transport would be.
    pub fn from_fixtures() -> Self {
        Self {
            recommender: crate::services::demo::sdk::finops::client(),
        }
    }
    pub async fn load_recommendations(
        &self,
        project: &str,
        recommender: RecommenderId,
        location: &str,
    ) -> Result<Vec<CostRecommendation>, ApiError> {
        use google_cloud_gax::paginator::ItemPaginator;
        let parent = format!(
            "projects/{project}/locations/{location}/recommenders/{}",
            recommender.id()
        );
        let mut rows = Vec::new();
        let mut items = self
            .recommender
            .list_recommendations()
            .set_parent(parent)
            .by_item();
        while let Some(recommendation) = items.next().await {
            rows.push(map_recommendation(&recommendation?));
        }
        Ok(rows)
    }
}

/// The priority enum's default, which stands for "none stated" and is not a
/// priority a card should print.
const UNSPECIFIED_PRIORITY: &str = "PRIORITY_UNSPECIFIED";

pub fn map_recommendation(
    recommendation: &google_cloud_recommender_v1::model::Recommendation,
) -> CostRecommendation {
    use google_cloud_recommender_v1::model::impact::Projection;

    // The API states a saving as a NEGATIVE cost over the projection's OWN
    // duration. Both are corrected here, exactly as on the CLI path, so no
    // render site has to remember either convention — and a projection whose
    // duration is missing yields `None` rather than an unscaled figure
    // presented as monthly.
    let monthly = recommendation
        .primary_impact
        .as_ref()
        .and_then(|impact| impact.projection.as_ref())
        .and_then(|projection| match projection {
            Projection::CostProjection(cost) => Some(cost),
            _ => None,
        })
        .and_then(|projection| {
            let cost = projection.cost.as_ref()?;
            let duration = projection.duration.as_ref()?.seconds();
            if duration == 0 {
                return None;
            }
            let money =
                Money::parse(&cost.units.to_string(), cost.nanos, &cost.currency_code).ok()?;
            money.negate().scaled(MONTH_SECONDS, i128::from(duration))
        });

    CostRecommendation {
        name: recommendation.name.clone(),
        description: recommendation.description.clone(),
        state: RecommendationState::from_api(
            recommendation
                .state_info
                .as_ref()
                .and_then(|info| info.state.name()),
        ),
        // `PRIORITY_UNSPECIFIED` is the enum's default, which is what a
        // recommendation carrying no priority arrives as. The CLI path omits
        // the field entirely in that case, so the card printed a bare cost;
        // passing the sentinel through printed `[PRIORITY_UNSPECIFIED]` beside
        // it instead of nothing.
        priority: recommendation
            .priority
            .name()
            .filter(|name| *name != UNSPECIFIED_PRIORITY)
            .map(|name| name.to_string()),
        xor_group: non_empty(&recommendation.xor_group_id),
        monthly,
        last_refresh: timestamp_to_string(recommendation.last_refresh_time.as_ref()),
        target_resources: recommendation
            .content
            .as_ref()
            .map(|content| {
                content
                    .operation_groups
                    .iter()
                    .flat_map(|group| group.operations.iter())
                    .filter(|operation| !operation.resource.is_empty())
                    .map(|operation| operation.resource.clone())
                    .collect()
            })
            .unwrap_or_default(),
    }
}
