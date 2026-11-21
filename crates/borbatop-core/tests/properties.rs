//! Property tests: the parsers never panic and keep their promises on any input.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use borbatop_core::{
    Bytes, CpuTicks, ElapsedTicks, Hundredths, Pid, SamplePair, UserTable, cpu_permille,
    order_as_tree, package_name_from_dir, parse_cgroup_unit, parse_cpu_stat, parse_diskstats,
    parse_loadavg, parse_meminfo, parse_mounts, parse_net_dev, parse_pressure, parse_process_stat,
    parse_systemctl_units, parse_upgrades, parse_uptime,
};
use proptest::prelude::*;

proptest! {
    #[test]
    fn no_parser_panics_on_arbitrary_text(text in "\\PC{0,400}") {
        let _ = parse_process_stat(&text);
        let _ = parse_cpu_stat(&text);
        let _ = parse_meminfo(&text);
        let _ = parse_loadavg(&text);
        let _ = parse_uptime(&text);
        let _ = parse_pressure(&text);
        let _ = parse_net_dev(&text);
        let _ = parse_diskstats(&text);
        let _ = parse_upgrades(&text);
        let _ = parse_cgroup_unit(&text);
        let _ = package_name_from_dir(&text);
        let _ = parse_mounts(&text);
        let _ = parse_systemctl_units(&text);
        let _ = UserTable::parse(&text);
    }

    #[test]
    fn a_well_formed_stat_line_round_trips_every_field(
        pid in 1_u32..4_000_000,
        parent in 0_u32..4_000_000,
        comm in "[ -~]{0,15}",
        user in 0_u64..u64::MAX / 2,
        system in 0_u64..u64::MAX / 2,
        threads in 1_u32..100_000,
        start in 0_u64..u64::MAX,
        rss in 0_i64..i64::MAX,
    ) {
        let line = format!(
            "{pid} ({comm}) S {parent} 1 1 0 -1 0 0 0 0 0 {user} {system} 0 0 20 0 {threads} 0 {start} 1 {rss} 0"
        );
        let sample = parse_process_stat(&line).unwrap();
        prop_assert_eq!(sample.pid(), Pid::new(pid));
        prop_assert_eq!(sample.parent(), Pid::new(parent));
        prop_assert_eq!(sample.comm(), comm.as_str());
        prop_assert_eq!(sample.cpu_ticks(), CpuTicks(user + system));
        prop_assert_eq!(sample.threads().get(), threads);
        prop_assert_eq!(sample.id().start_time().get(), start);
        prop_assert_eq!(i64::try_from(sample.resident_pages().get()).unwrap(), rss);
    }

    #[test]
    fn hundredths_round_trip_through_text(whole in 0_u32..1_000_000, fraction in 0_u32..100) {
        let text = format!("{whole}.{fraction:02}");
        let parsed = Hundredths::parse(&text).unwrap();
        prop_assert_eq!(parsed.get(), whole * 100 + fraction);
        prop_assert_eq!(parsed.to_string(), text);
    }

    #[test]
    fn byte_formatting_is_total_and_share_is_bounded(value in any::<u64>(), total in any::<u64>()) {
        let text = Bytes::new(value).to_string();
        prop_assert!(!text.is_empty());
        prop_assert!(Bytes::new(value).permille_of(Bytes::new(total)) <= 1_000);
    }

    #[test]
    fn cpu_usage_equals_the_scaled_delta_whenever_it_fits(
        previous in 0_u64..1_000_000_000,
        delta in 0_u64..1_000_000_000,
        elapsed in 1_u64..1_000_000,
    ) {
        let window = SamplePair::new(CpuTicks(previous), CpuTicks(previous + delta));
        let usage = cpu_permille(window, ElapsedTicks(elapsed)).unwrap();
        prop_assert_eq!(usage.get(), delta * 1_000 / elapsed);
    }

    #[test]
    fn counters_that_go_backwards_are_always_rejected(
        previous in 1_u64..u64::MAX,
        drop_by in 1_u64..1_000,
        elapsed in 1_u64..1_000,
    ) {
        let current = previous.saturating_sub(drop_by.min(previous));
        prop_assume!(current < previous);
        let window = SamplePair::new(CpuTicks(previous), CpuTicks(current));
        prop_assert!(cpu_permille(window, ElapsedTicks(elapsed)).is_err());
    }

    #[test]
    fn the_tree_lists_every_process_once_and_parents_first(
        parents in proptest::collection::vec(0_u32..60, 0..60),
    ) {
        let links: Vec<(Pid, Pid)> = parents
            .iter()
            .enumerate()
            .map(|(index, parent)| (Pid::new(u32::try_from(index).unwrap() + 1), Pid::new(*parent)))
            .collect();

        let rows = order_as_tree(&links);

        let mut indices: Vec<usize> = rows.iter().map(borbatop_core::TreeRow::index).collect();
        indices.sort_unstable();
        prop_assert_eq!(indices, (0..links.len()).collect::<Vec<_>>());

        let position_of: std::collections::HashMap<usize, usize> =
            rows.iter().enumerate().map(|(position, row)| (row.index(), position)).collect();
        for row in &rows {
            let (pid, parent) = links[row.index()];
            if parent == pid {
                continue;
            }
            if let Some(parent_index) = links.iter().position(|(candidate, _)| *candidate == parent) {
                let parent_row = rows[position_of[&parent_index]];
                if parent_row.depth() < row.depth() {
                    prop_assert!(position_of[&parent_index] < position_of[&row.index()]);
                }
            }
        }
    }

    #[test]
    fn mount_points_with_escaped_spaces_decode(name in "[a-z]{1,8}( [a-z]{1,8}){0,2}") {
        let escaped = name.replace(' ', "\\040");
        let text = format!("/dev/sda1 /mnt/{escaped} ext4 rw 0 0\n");
        let mounts = parse_mounts(&text);
        prop_assert_eq!(mounts.len(), 1);
        prop_assert_eq!(mounts[0].mount_point(), format!("/mnt/{name}"));
    }
}
