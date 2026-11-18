# nuvem-domain

The pure domain logic behind [nuvem](https://github.com/filipeaaoliveira/nuvem),
a terminal UI for Google Cloud: identifiers, status enums, the filter grammar,
money and text helpers, and the sanitisation of `gcloud` output.

It is published so that `nuvem` can be installed with `cargo install nuvem`.
It has no API stability guarantee of its own: it moves in lockstep with
`nuvem`, and a minor release of either may change it. Depend on it directly
at your own risk.

Licensed under the [MIT License](LICENSE).
