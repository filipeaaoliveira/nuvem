//! Whether a value is safe to hand a child process as an argument.
//!
//! Every launch nuvem performs interpolates values it did not author —
//! instance names, zones, cluster names, projects — into an argv. A value
//! beginning with `-` stops being data at that point: `gcloud`, `ssh` and
//! `kubectl` all read it as an option, so an instance called `--zone` turns a
//! resource reference into a flag the operator never typed.
//!
//! There is no escape to apply. The usual end-of-options marker is unavailable
//! here: gcloud spells `--` as "pass everything after this to ssh"
//! (`gcloud compute ssh … -- -N -L …`, see [`crate::gcp::tunnel`]), so it
//! cannot also mean "stop parsing options". The remaining move is to refuse,
//! which is why this module offers a predicate rather than a sanitiser.
//!
//! Refusals belong at the point a launch is *offered*, not in the argv
//! builders: those are called a second time from the render path to draw the
//! confirmation preview, and a builder that could fail would have to fail
//! there too, where there is nothing useful to do about it. Offering is also
//! where the operator can be told why, and it is where every neighbouring
//! refusal in this codebase already lives.
//!
//! This is defence in depth, not a boundary check. GCP resource names must
//! begin with a letter, so a well-behaved backend cannot produce one of these;
//! what this catches is a parse that went wrong, a future caller that reaches
//! these builders from somewhere less trusted, and output from a `gcloud` that
//! is not the one we think it is.

/// The first value a child would read as an option instead of as data, if any.
///
/// Returns the offender rather than a bare `bool` so the caller can name it:
/// "refusing to tunnel" is a puzzle to an operator, and "refusing to tunnel:
/// `-x` would be read as an option" is a bug report about their environment.
pub fn option_like_value<'a>(values: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    values.into_iter().find(|value| value.starts_with('-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_values_are_all_data() {
        assert_eq!(
            option_like_value(["web-1", "us-central1-a", "my-project"]),
            None
        );
    }

    /// The dash that matters is the leading one. GCP names are full of
    /// interior dashes, and refusing those would refuse nearly every real
    /// resource.
    #[test]
    fn an_interior_dash_is_not_an_option() {
        assert_eq!(option_like_value(["europe-west1-b"]), None);
    }

    #[test]
    fn a_leading_dash_is_reported_by_value() {
        assert_eq!(option_like_value(["web-1", "--zone"]), Some("--zone"));
    }

    /// A single dash is `stdin` to many tools and an option prefix to the
    /// rest; either way it is not a resource name.
    #[test]
    fn a_lone_dash_is_refused_too() {
        assert_eq!(option_like_value(["-"]), Some("-"));
    }

    /// The *first* offender is reported, so the message names the value the
    /// operator will find first when reading their own row.
    #[test]
    fn the_first_offender_wins() {
        assert_eq!(option_like_value(["-a", "-b"]), Some("-a"));
    }

    #[test]
    fn an_empty_value_is_not_option_like() {
        // Emptiness is a different refusal, made by the callers that care
        // (`resolve_launch_target` has always refused empty names); this
        // predicate must not claim it.
        assert_eq!(option_like_value([""]), None);
    }
}
