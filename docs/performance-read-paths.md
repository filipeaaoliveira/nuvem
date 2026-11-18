# Read-path performance on the native clients

`docs/decisions/0001-native-sdk-for-read-paths.md` moved every listing and
every detail load off the `gcloud` subprocess and onto the generated client
libraries. It was taken on two captures of a single path — `gcloud storage
buckets list --format=json` at 1.32s – 1.56s against the same data over HTTPS
with a cached token at 0.31s – 0.47s — and it said so. This page is the
measurement that record was missing: every read path, on both backends,
against a live project.

It is also the review that found seven defects, which is the more useful half
of the result. They are listed at the end.

## How these numbers were produced

`tests/read_path_bench.rs` measures each path through the application's own
service code rather than through a hand-built call, so what it times is what an
operator waits for. Both backends are aimed at the same configuration, the same
project and the same parameters; both are warmed by one untimed call, so no
timed sample pays a connection setup and no native sample contains the
`config-helper` subprocess that mints the token; iterations interleave native,
CLI, native, CLI, so a machine that gets busier partway through slows both
equally. Failures and skips are reported per path and never end the run.

```text
READ_PATH_BENCH_ITERATIONS=5 cargo nextest run --release \
    -E 'test(read_path_bench)' --run-ignored all --no-capture
```

It reads. It creates nothing, changes nothing and deletes nothing, and the one
Secret Manager call it makes is `secrets.list`, never `access`. It prints
timings and row counts only: no project id, bucket, account, instance,
configuration, region or zone reaches stdout, in the table or in an error.

Two runs are reported, taken minutes apart with five iterations per backend per
path, on a quiet machine and after every fix below. Two rather than one because
the agreement between them is what says a gap is a property of the backend
rather than of the machine that afternoon.

## Fixed costs

These are not per path. They are paid once, and they are why the per-path
numbers look the way they do.

| cost | run 1 | run 2 |
| --- | ---: | ---: |
| `gcloud --version`, the interpreter floor | 249 / 252 / 259 ms | 247 / 255 / 260 ms |
| native client construction, cold cache | 455 ms | 454 ms |
| first native request after construction | 471 ms | 526 ms |
| warm native requests on the same client | 317 / 324 / 327 ms | 327 / 348 / 349 ms |

And the one that answers the poll-cadence question, measured separately as
processor time rather than elapsed time, because a poll runs off-screen and
what it spends is the machine's rather than the operator's:

| processor time per poll tick | run 1 | run 2 |
| --- | ---: | ---: |
| native | 1.05 ms | 1.05 ms |
| cli | 376 ms | 381 ms |

`gcloud --version` moves no data at all. Whatever it costs is what every CLI row
pays before its own request begins. A quarter of a second is its floor on an
idle machine; under load earlier the same afternoon the same measurement read
985 / 1790 / 2182 ms, while a warm native request did not move. The CLI's cost
is not only larger, it is the part that degrades when the machine is busy.

Client construction is a cold-cache cost the session pays once per
configuration, and the client cache keeps it that way, so it is charged to the
first pane opened after a context switch rather than to the poll.

## Per-path results

Timings are min / median / max wall milliseconds per call. `cli / native` is the
ratio of the medians; above 1.0 the native path is faster.

### Run 1 (load 1.96 to 2.69)

| path | native ms | rows | cli ms | rows | cli / native |
| --- | ---: | ---: | ---: | ---: | ---: |
| storage buckets | 262 / 267 / 300 | 14 | 789 / 916 / 1130 | 14 | 3.4x |
| storage object page | 103 / 105 / 122 | 3 | 750 / 799 / 888 | 1 | 7.6x |
| compute instances | 977 / 999 / 1232 | 6 | 1339 / 1457 / 1785 | 6 | 1.5x |
| persistent disks | 424 / 519 / 623 | 18 | 901 / 987 / 2549 | 18 | 1.9x |
| gke clusters | 461 / 522 / 614 | 1 | 1041 / 1054 / 1096 | 1 | 2.0x |
| cloudsql instances | 112 / 132 / 250 | 1 | 499 / 556 / 616 | 1 | 4.2x |
| secrets | 141 / 184 / 187 | 0 | 611 / 638 / 707 | 0 | 3.5x |
| pubsub topics | 463 / 1046 / 1111 | 1 | 1821 / 2121 / 2331 | 1 | 2.0x |
| pubsub subscriptions | 292 / 468 / 603 | 0 | 998 / 1292 / 1805 | 0 | 2.8x |
| iam service accounts | 154 / 171 / 660 | 84 | 969 / 1040 / 1054 | 84 | 6.1x |
| iam policy members | 507 / 821 / 914 | 239 | 685 / 1084 / 1218 | 239 | 1.3x |
| cloud run services | 543 / 615 / 715 | 0 | 746 / 779 / 878 | 0 | 1.3x |
| cloud functions | 526 / 958 / 980 | 0 | 936 / 1317 / 1393 | 0 | 1.4x |
| logging read_recent | 764 / 831 / 943 | 50 | 1193 / 1294 / 1351 | 50 | 1.6x |
| finops recommendations | skipped | - | skipped | - | - |
| discovery projects | 426 / 985 / 1024 | 309 | 807 / 859 / 1260 | 305 | 0.9x |
| discovery regions | 599 / 681 / 760 | 43 | 1413 / 1449 / 2634 | 43 | 2.1x |
| discovery zones | 393 / 543 / 681 | 130 | 841 / 887 / 907 | 130 | 1.6x |
| detail: compute instance | 157 / 193 / 250 | 1 | 627 / 703 / 866 | 1 | 3.6x |
| detail: gke cluster | 105 / 122 / 259 | 1 | 653 / 688 / 712 | 1 | 5.6x |
| detail: cloudsql instance | 115 / 213 / 296 | 1 | 544 / 600 / 715 | 1 | 2.8x |
| detail: secret | skipped | - | skipped | - | - |
| detail: storage bucket | 98 / 101 / 111 | 1 | 514 / 529 / 596 | 1 | 5.2x |
| detail: cloud run service | skipped | - | skipped | - | - |

One median call per path: native 10479 ms against CLI 21053 ms, a factor of 2.0.

### Run 2 (load 2.42 to 2.00)

| path | native ms | rows | cli ms | rows | cli / native |
| --- | ---: | ---: | ---: | ---: | ---: |
| storage buckets | 286 / 300 / 317 | 14 | 776 / 793 / 904 | 14 | 2.6x |
| storage object page | 106 / 110 / 158 | 3 | 742 / 777 / 785 | 1 | 7.1x |
| compute instances | 902 / 996 / 1136 | 6 | 1357 / 1498 / 1563 | 6 | 1.5x |
| persistent disks | 436 / 468 / 624 | 18 | 909 / 923 / 973 | 18 | 2.0x |
| gke clusters | 465 / 520 / 621 | 1 | 995 / 1075 / 1106 | 1 | 2.1x |
| cloudsql instances | 100 / 202 / 215 | 1 | 496 / 687 / 834 | 1 | 3.4x |
| secrets | 120 / 158 / 184 | 0 | 510 / 566 / 599 | 0 | 3.6x |
| pubsub topics | 586 / 789 / 971 | 1 | 1490 / 1870 / 2349 | 1 | 2.4x |
| pubsub subscriptions | 222 / 607 / 716 | 0 | 914 / 1187 / 1461 | 0 | 2.0x |
| iam service accounts | 186 / 214 / 665 | 84 | 940 / 954 / 980 | 84 | 4.5x |
| iam policy members | 279 / 385 / 911 | 239 | 690 / 1181 / 1214 | 239 | 3.1x |
| cloud run services | 475 / 539 / 549 | 0 | 781 / 884 / 929 | 0 | 1.6x |
| cloud functions | 927 / 949 / 1131 | 0 | 980 / 1364 / 1409 | 0 | 1.4x |
| logging read_recent | 724 / 749 / 1447 | 50 | 1240 / 1283 / 1364 | 50 | 1.7x |
| finops recommendations | skipped | - | skipped | - | - |
| discovery projects | 975 / 1047 / 1425 | 309 | 844 / 948 / 1299 | 305 | 0.9x |
| discovery regions | 659 / 703 / 1674 | 43 | 1389 / 1469 / 1555 | 43 | 2.1x |
| discovery zones | 296 / 325 / 358 | 130 | 874 / 880 / 951 | 130 | 2.7x |
| detail: compute instance | 164 / 173 / 272 | 1 | 650 / 685 / 802 | 1 | 4.0x |
| detail: gke cluster | 94 / 129 / 211 | 1 | 609 / 614 / 680 | 1 | 4.8x |
| detail: cloudsql instance | 108 / 215 / 238 | 1 | 576 / 609 / 634 | 1 | 2.8x |
| detail: secret | skipped | - | skipped | - | - |
| detail: storage bucket | 101 / 108 / 136 | 1 | 505 / 531 / 536 | 1 | 4.9x |
| detail: cloud run service | skipped | - | skipped | - | - |

One median call per path: native 9686 ms against CLI 20779 ms, a factor of 2.1.

Three paths were skipped in both runs for reasons about the project rather than
about either backend: the recommender needs a zone or region the active
configuration does not set, and the Secret and Cloud Run detail panes need a row
their own listing did not return.

## What the numbers say

**The decision holds, and the path it was taken on reproduces.** The bucket
listing was 1.32s – 1.56s on `gcloud` against 0.31s – 0.47s natively when the
record was written; here it is 0.79 – 0.92s against 0.27 – 0.30s. The absolute
numbers moved, the shape did not.

**Detail panes gained the most, which is where it is felt.** Every pane measured
lands between 2.8x and 5.6x, at roughly 100 – 215 ms against 530 – 700 ms. The
record argued this in advance: a listing repeats on a timer nobody watches,
while a `describe` runs with an operator waiting for the pane to open. A pane
that opens in a tenth of a second reads as instant; three-quarters of a second
reads as a wait.

**The aggregate gain is about 2x, not the 4x one path suggested.** The whole
read surface costs 9.7 – 10.5 seconds of median call time natively against
20.8 – 21.1 seconds on the CLI. That is the figure to quote for this migration.

**The interpreter floor is most of the CLI's cost on small answers and almost
none of it on large ones.** Where a path returns little, the CLI's time is mostly
`gcloud` starting: the bucket listing, the object page, Cloud SQL and Secret
Manager all sit between 2.6x and 7.6x on answers of at most fourteen rows.
Where a path returns a lot, the request dominates and the two converge: 309
projects, 239 policy members and 84 service accounts are the three flattest rows
on the page. Pub/Sub topics is expensive on both backends for the same reason —
the count column costs a second listing — and lands at 2.0x – 2.4x now that both
backends pay for it.

**One path is not faster natively.** The project listing came back at 0.9x in
both runs: 309 projects is the largest answer measured, the request dominates,
and the native path pays a search call where `gcloud projects list` pays a plain
one. IAM policy members swings between 1.3x and 3.1x — the native path expands
239 bindings client-side, and that expansion costs about what the interpreter
start saves. Cloud Functions and Cloud Run services sit at 1.3x – 1.6x, close
enough that load moves them. Nothing here needs fixing; nothing here should be
advertised as a speedup either.

**Native pagination returns a real page.** The object browser's page holds three
rows natively against one on the CLI, because `gcloud storage objects list`
emits no page token and that path cannot ask for more. This is the pagination
consequence the record predicted, visible as a row count rather than as a
timing — the two columns on that row are not measuring the same work.

## What measuring found

Seven divergences, all of them silent, all now fixed. Six were found by reading
the two implementations against each other after the seventh made it obvious
that the reading was worth doing.

- **Pub/Sub topics never showed a subscription count.** The CLI path listed
  subscriptions and joined them to fill the column; the native path returned
  topics alone, so the column read as unknown on every native load. This is the
  one the benchmark caught directly: it was the widest gap on the page at 21.7x,
  because the two backends were not doing the same work.
- **Cloud SQL listed only the first page.** Every other native lister drains a
  paginator; this one read a single response and dropped the page token, so a
  project past one page of instances was quietly short.
- **The log tail took the newest entries rather than the oldest.** The CLI asks
  for ascending order from the cursor; the native path asked for descending. With
  more entries between polls than the cap, the window skipped the middle, the
  cursor advanced past it, and nothing counted the loss.
- **The IAM policy was fetched at a version that cannot carry conditions.** A
  conditional binding came back stripped of its condition, so a row claimed a
  broader grant than the one that exists.
- **Region and zone status rendered as a number.** `1` instead of `UP`, in a
  picker whose incremental filter matches what is painted.
- **Cloud Run lost four fields:** the reason a condition failed, the readiness
  condition itself, the `LATEST` marker on a traffic split, and the ingress
  setting in the vocabulary the pane prints.
- **Storage showed a nameless zero-byte row** for the folder placeholder object
  the CLI path drops.

Each fix carries a test. The shape they share is worth stating: every one of
them is a value the old backend derived — with a second call, an argument, a
translation or a filter — that the new backend had no reason to reproduce,
because nothing in the row type says where a field came from. A migration
checklist that asks "does every path return rows?" passes all seven.

## The poll cadence, which the record left open

The decision record named an experiment and did not run it: raising
`poll_interval_secs` from 5 to 30 would cut listing volume roughly six-fold
with a one-line change, and if the cadence were the real cost then that was
cheaper than migrating a backend. It has now been run, and it is answered by
the processor-time rows above rather than by the wall-clock table, because a
poll tick happens while nobody is waiting for it. What it costs is CPU.

At the default cadence the active table is re-listed 720 times an hour. One
tick of the cheapest listing costs:

| per hour of an idle session | at 5s | at 30s |
| --- | ---: | ---: |
| cli backend | 4.7 CPU-minutes | 47 CPU-seconds |
| native backend | 0.7 CPU-seconds | 0.1 CPU-seconds |

The experiment's premise was true and is now obsolete. On the CLI backend a
tick cost 376 – 392 ms of processor time, nearly all of it starting a Python
interpreter, and lengthening the interval would have reclaimed about four CPU
minutes per hour -- a real saving, and the reason the question was worth
asking. On the native backend a tick costs about one millisecond, so the same
change reclaims about half a second per hour. Four hundred times less work per
tick is what the migration bought, and it bought it without making the rows
staler.

So the item is closed rather than deferred: the cadence stays at 5 seconds,
because the thing it was going to pay for no longer costs anything, and
freshness does. What remains is request volume rather than machine cost -- 720
listings an hour against a quota -- and that is an argument for backing off
when a table is not on screen, which is a different change from a longer
interval and is not made here.

## What else measuring found

The page size is part of the bound, not a detail of it. One of the listings
capped in this pass got materially faster rather than merely bounded: IAM
service accounts went from 697 ms to 171 – 214 ms for the same 84 rows, because
without a stated page size the API sends its default of twenty and the
paginator was making five round trips where one would do. The same listing
then failed outright when it was first asked for 500 -- `serviceAccounts.list`
documents a maximum of 100 and rejects anything larger instead of clamping, so
the table came back empty. Both of those were caught by re-running this
benchmark rather than by any test, which is the argument for keeping it in the
tree.
