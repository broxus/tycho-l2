#!/usr/bin/env python3
import sys
from dataclasses import dataclass
from typing import List, Union

from dashboard_builder import (
    Layout,
    Expr,
    stat_panel,
    target,
    template,
    timeseries_panel,
)
from grafanalib import _gen, formatunits as UNITS
from grafanalib.core import (
    Annotations,
    Dashboard,
    GRAPH_TOOLTIP_MODE_SHARED_CROSSHAIR,
    Panel,
    RowPanel,
    StatValueMappingItem,
    StatValueMappings,
    Target,
    Template,
    Templating,
)


@dataclass(frozen=True)
class FullWidth:
    panel: Panel


def generate_legend_format(labels: List[str]) -> str:
    legend_format = ""
    for label in labels:
        key = label.split("=")[0]
        legend_format += f" {{{{{key}}}}}"
    return legend_format.strip()


def normalize_expr(
    expr: Union[str, Expr],
    labels: List[str] | None = None,
) -> Expr:
    if isinstance(expr, Expr):
        return expr.extra(default_label_selectors=[])
    return Expr(metric=expr, label_selectors=labels or [], default_label_selectors=[])


def create_gauge_panel(
    expr: Union[str, List[Union[str, Expr]], Expr],
    title: str,
    unit_format=UNITS.NUMBER_FORMAT,
    labels: List[str] | None = None,
    legend_format: str | None = None,
    legend_placement: str = "right",
) -> Panel:
    labels = labels or []
    if isinstance(expr, str):
        expr = [normalize_expr(expr, labels)]
    elif isinstance(expr, list):
        expr = [normalize_expr(item, labels) for item in expr]
    elif isinstance(expr, Expr):
        expr = [normalize_expr(expr)]
    else:
        raise TypeError(
            "expr must be a string, a list of strings, or a list of Expr objects."
        )

    if legend_format is None:
        legend_format = generate_legend_format(labels)

    targets = [target(item, legend_format=legend_format) for item in expr]

    return timeseries_panel(
        title=title,
        targets=targets,
        unit=unit_format,
        legend_placement=legend_placement,
    )


def create_stat_panel(
    expr: str,
    title: str,
    *,
    legend_format: str,
    unit_format: str = UNITS.NUMBER_FORMAT,
    mappings: StatValueMappings | None = None,
) -> Panel:
    return stat_panel(
        title=title,
        targets=[
            Target(
                expr=expr,
                legendFormat=legend_format,
                instant=True,
                datasource="${source}",
            )
        ],
        format=unit_format,
        mappings=mappings,
        text_mode="value_and_name",
    )


def create_row(
    name: str, metrics, repeat: str | None = None, collapsed: bool = True
) -> RowPanel:
    layout = Layout(name, repeat=repeat, collapsed=collapsed)

    half_width_panels = []

    def flush_half_width_panels() -> None:
        for i in range(0, len(half_width_panels), 2):
            layout.row(half_width_panels[i : i + 2])
        half_width_panels.clear()

    for panel in metrics:
        is_full = isinstance(panel, FullWidth)
        real_panel = panel.panel if is_full else panel

        if is_full:
            flush_half_width_panels()
            layout.row([real_panel])
            continue

        half_width_panels.append(real_panel)

    flush_half_width_panels()

    return layout.row_panel


def overview() -> RowPanel:
    labels = ['src=~"$src"', 'dst=~"$dst"']
    status_mappings = StatValueMappings(
        StatValueMappingItem("Initializing", "0", "blue"),
        StatValueMappingItem("Running", "1", "green"),
        StatValueMappingItem("Retrying", "2", "red"),
    )

    metrics = [
        create_stat_panel(
            'max by (src, dst) (sync_uploader_status{src=~"$src", dst=~"$dst"})',
            "Uploader Status",
            legend_format="{{src}} -> {{dst}}",
            mappings=status_mappings,
        ),
        create_stat_panel(
            'max by (src, dst) (sync_uploader_last_checked_vset{src=~"$src", dst=~"$dst"})',
            "Last Checked Vset",
            legend_format="{{src}} -> {{dst}}",
        ),
        create_stat_panel(
            'max by (src, dst) (sync_uploader_last_seen_src_key_block_seqno{src=~"$src", dst=~"$dst"})',
            "Last Seen Src Key Block",
            legend_format="{{src}} -> {{dst}}",
        ),
        create_stat_panel(
            'max by (src, dst) (sync_uploader_last_sent_key_block_seqno{src=~"$src", dst=~"$dst"})',
            "Last Sent Key Block",
            legend_format="{{src}} -> {{dst}}",
        ),
        create_gauge_panel(
            Expr(
                metric='clamp_min(time() - sync_uploader_last_success_unix_time{src=~"$src", dst=~"$dst"}, 0) unless sync_uploader_last_success_unix_time{src=~"$src", dst=~"$dst"} == 0',
                default_label_selectors=[],
            ),
            "Seconds Since Last Success",
            unit_format=UNITS.SECONDS,
            labels=labels,
        ),
        create_gauge_panel(
            Expr(
                metric='clamp_min(time() - sync_uploader_last_error_unix_time{src=~"$src", dst=~"$dst"}, 0) unless sync_uploader_last_error_unix_time{src=~"$src", dst=~"$dst"} == 0',
                default_label_selectors=[],
            ),
            "Seconds Since Last Error",
            unit_format=UNITS.SECONDS,
            labels=labels,
        ),
    ]

    return create_row("Overview", metrics)


def wallet() -> RowPanel:
    wallet_labels = ['src=~"$src"', 'dst=~"$dst"', 'wallet=~"$wallet"']

    metrics = [
        FullWidth(
            timeseries_panel(
                title="Wallet Balance",
                targets=[
                    target(
                        normalize_expr("sync_uploader_wallet_balance", wallet_labels),
                        legend_format="{{src}} -> {{dst}} {{wallet}} balance",
                    ),
                    target(
                        normalize_expr(
                            "sync_uploader_wallet_min_required_balance", wallet_labels
                        ),
                        legend_format="{{src}} -> {{dst}} {{wallet}} min required",
                    ),
                ],
                unit=UNITS.NUMBER_FORMAT,
                legend_placement="bottom",
            )
        ),
        create_gauge_panel(
            Expr(
                metric='sync_uploader_wallet_balance{src=~"$src", dst=~"$dst", wallet=~"$wallet"} - sync_uploader_wallet_min_required_balance{src=~"$src", dst=~"$dst", wallet=~"$wallet"}',
                default_label_selectors=[],
            ),
            "Wallet Headroom",
            labels=wallet_labels,
            legend_format="{{src}} -> {{dst}} {{wallet}}",
        ),
    ]

    return create_row("Wallet", metrics)


def bridge_progress() -> RowPanel:
    labels = ['src=~"$src"', 'dst=~"$dst"']

    metrics = [
        FullWidth(
            timeseries_panel(
                title="Key Block Sequence Numbers",
                targets=[
                    target(
                        normalize_expr(
                            "sync_uploader_last_seen_src_key_block_seqno", labels
                        ),
                        legend_format="{{src}} -> {{dst}} last seen",
                    ),
                    target(
                        normalize_expr(
                            "sync_uploader_last_sent_key_block_seqno", labels
                        ),
                        legend_format="{{src}} -> {{dst}} last sent",
                    ),
                ],
                unit=UNITS.NUMBER_FORMAT,
                legend_placement="bottom",
            )
        ),
        create_gauge_panel(
            "sync_uploader_cached_key_blocks",
            "Cached Key Blocks",
            labels=labels,
        ),
        create_gauge_panel(
            "sync_uploader_min_bridge_state_lt",
            "Minimum Bridge State LT",
            labels=labels,
        ),
        create_gauge_panel(
            "sync_uploader_last_checked_vset",
            "Current Checked Vset",
            labels=labels,
        ),
        create_gauge_panel(
            "sync_uploader_last_sent_key_block_utime",
            "Last Sent Key Block Utime",
            labels=labels,
        ),
    ]

    return create_row("Bridge Progress", metrics)


def health() -> RowPanel:
    labels = ['src=~"$src"', 'dst=~"$dst"']

    metrics = [
        FullWidth(
            timeseries_panel(
                title="Uploader Status History",
                targets=[
                    target(
                        normalize_expr("sync_uploader_status", labels),
                        legend_format="{{src}} -> {{dst}}",
                    )
                ],
                unit=UNITS.NUMBER_FORMAT,
                legend_placement="bottom",
            )
        ),
        FullWidth(
            timeseries_panel(
                title="Event Unix Timestamps",
                targets=[
                    target(
                        normalize_expr("sync_uploader_last_success_unix_time", labels),
                        legend_format="{{src}} -> {{dst}} last success",
                    ),
                    target(
                        normalize_expr("sync_uploader_last_error_unix_time", labels),
                        legend_format="{{src}} -> {{dst}} last error",
                    ),
                ],
                unit=UNITS.NUMBER_FORMAT,
                legend_placement="bottom",
            )
        ),
    ]

    return create_row("Health", metrics)


def templates() -> Templating:
    return Templating(
        list=[
            Template(
                name="source",
                query="prometheus",
                type="datasource",
            ),
            template(
                name="src",
                query="label_values(sync_uploader_status, src)",
                data_source="${source}",
                hide=0,
                regex=None,
                multi=True,
                include_all=True,
                all_value=".*",
            ),
            template(
                name="dst",
                query='label_values(sync_uploader_status{src=~"$src"}, dst)',
                data_source="${source}",
                hide=0,
                regex=None,
                multi=True,
                include_all=True,
                all_value=".*",
            ),
            template(
                name="wallet",
                query='label_values(sync_uploader_wallet_balance{src=~"$src", dst=~"$dst"}, wallet)',
                data_source="${source}",
                hide=0,
                regex=None,
                multi=True,
                include_all=True,
                all_value=".*",
            ),
        ]
    )


dashboard = Dashboard(
    "Tycho L2 Sync Service Metrics",
    templating=templates(),
    refresh="30s",
    panels=[
        overview(),
        wallet(),
        bridge_progress(),
        health(),
    ],
    annotations=Annotations(),
    uid="tychol2syncsvc",
    version=1,
    schemaVersion=14,
    graphTooltip=GRAPH_TOOLTIP_MODE_SHARED_CROSSHAIR,
    timezone="browser",
).auto_panel_ids()


if len(sys.argv) > 1:
    stream = open(sys.argv[1], "w")
else:
    stream = sys.stdout

_gen.write_dashboard(dashboard, stream)
