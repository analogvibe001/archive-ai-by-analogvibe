//! Port of src/core/dateExtractor.js

use crate::scanner::{DateInfo, FileNode, TreeNode};
use std::collections::HashMap;

pub const MONTHS_ES: [&str; 12] = [
    "Enero", "Febrero", "Marzo", "Abril", "Mayo", "Junio",
    "Julio", "Agosto", "Septiembre", "Octubre", "Noviembre", "Diciembre",
];

pub fn month_name(num: u32) -> &'static str {
    if (1..=12).contains(&num) {
        MONTHS_ES[(num - 1) as usize]
    } else {
        "Desconocido"
    }
}

pub fn pad_day(day: Option<u32>) -> Option<String> {
    day.map(|d| format!("{:02}", d))
}

/// bestDate(node) — priority: 1) folder-name date, 2) majority vote from
/// node.dates, 3) null (caller falls back to current year elsewhere).
pub fn best_date(node: &TreeNode) -> Option<DateInfo> {
    if let Some(nd) = &node.name_date {
        if nd.year != 0 {
            let mut d = nd.clone();
            d.source = "folder_name".to_string();
            d.confidence = Some(90);
            return Some(d);
        }
    }

    if !node.dates.is_empty() {
        #[derive(Clone)]
        struct Vote {
            year: i32,
            month: Option<u32>,
            count: i32,
        }
        let mut votes: HashMap<(i32, Option<u32>), Vote> = HashMap::new();
        for d in &node.dates {
            if d.year < 2010 || d.year > 2030 {
                continue;
            }
            let key = (d.year, d.month);
            let entry = votes.entry(key).or_insert(Vote { year: d.year, month: d.month, count: 0 });
            entry.count += 1;
        }
        let mut sorted: Vec<&Vote> = votes.values().collect();
        sorted.sort_by(|a, b| b.count.cmp(&a.count));
        if let Some(best) = sorted.first() {
            let conf = std::cmp::min(85, 40 + best.count * 5);
            let mut day_votes: HashMap<u32, i32> = HashMap::new();
            for d in &node.dates {
                if d.year == best.year && d.month == best.month {
                    if let Some(day) = d.day {
                        *day_votes.entry(day).or_insert(0) += 1;
                    }
                }
            }
            let top_day = day_votes.iter().max_by_key(|(_, c)| **c).map(|(d, _)| *d);
            return Some(DateInfo {
                year: best.year,
                month: best.month,
                day: top_day,
                source: "file_dates".to_string(),
                confidence: Some(conf),
            });
        }
    }

    None
}

/// bestDateForFile — priority: filename date > first date found among
/// ancestor folder names > file mtime.
pub fn best_date_for_file(file: &FileNode, ancestor_dates: &[DateInfo]) -> Option<DateInfo> {
    if let Some(nd) = &file.name_date {
        if nd.year != 0 {
            let mut d = nd.clone();
            d.source = "file_name".to_string();
            d.confidence = Some(92);
            return Some(d);
        }
    }
    for ad in ancestor_dates {
        if ad.year != 0 {
            let mut d = ad.clone();
            d.source = "folder_name".to_string();
            d.confidence = Some(75);
            return Some(d);
        }
    }
    if file.mtime_date.year != 0 {
        let mut d = file.mtime_date.clone();
        d.source = "mtime".to_string();
        d.confidence = Some(45);
        return Some(d);
    }
    None
}
