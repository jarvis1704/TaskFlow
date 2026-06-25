use chrono::{Local, Datelike, Timelike, NaiveDate, NaiveTime, Weekday};

/// Parse a task input string to extract title, due date, and reminder time.
/// Supports formats like:
/// - "Buy groceries tomorrow at 5pm"
/// - "Call dentist on 2026-07-01 at 14:30"
/// - "Do laundry Friday 10:00"
/// - "Meeting at 9am"
pub fn parse_task_input(input: &str) -> (String, Option<NaiveDate>, Option<NaiveTime>) {
    let words: Vec<&str> = input.split_whitespace().collect();
    let mut clean_words = Vec::new();
    let mut due_date = None;
    let mut reminder_time = None;

    let today = Local::now().date_naive();
    let mut skip_next = 0;

    for i in 0..words.len() {
        if skip_next > 0 {
            skip_next -= 1;
            continue;
        }

        let word = words[i];
        let word_lower = word.to_lowercase();

        // Check if this word is a preposition that might precede a date or time
        let is_preposition = word_lower == "on" || word_lower == "at" || word_lower == "by" || word_lower == "due";

        let mut parsed_date = None;
        let mut parsed_time = None;
        let mut consumed_words = 0;

        // Try lookahead only if the current word is a preposition and there's a next word
        if is_preposition && i + 1 < words.len() {
            let next_word = words[i + 1];
            let next_word_lower = next_word.to_lowercase();

            if let Some(date) = parse_date_token(&next_word_lower, today) {
                // Only consume lookahead if due_date is not yet set
                if due_date.is_none() {
                    parsed_date = Some(date);
                    consumed_words = 1;
                }
            } else if let Some(time) = parse_time_token(&next_word_lower) {
                // Only consume lookahead if reminder_time is not yet set
                if reminder_time.is_none() {
                    parsed_time = Some(time);
                    consumed_words = 1;
                }
            }
        }

        // Try parsing the current word directly if no lookahead matched
        if parsed_date.is_none() && parsed_time.is_none() {
            if due_date.is_none() {
                if let Some(date) = parse_date_token(&word_lower, today) {
                    parsed_date = Some(date);
                    consumed_words = 0;
                }
            }
            if parsed_date.is_none() && reminder_time.is_none() {
                if let Some(time) = parse_time_token(&word_lower) {
                    parsed_time = Some(time);
                    consumed_words = 0;
                }
            }
        }

        if let Some(date) = parsed_date {
            due_date = Some(date);
            if is_preposition && consumed_words > 0 {
                skip_next = consumed_words;
            } else {
                skip_next = consumed_words;
            }
        } else if let Some(time) = parsed_time {
            reminder_time = Some(time);
            if is_preposition && consumed_words > 0 {
                skip_next = consumed_words;
            } else {
                skip_next = consumed_words;
            }
        } else {
            clean_words.push(word);
        }
    }

    let clean_title = clean_words.join(" ");
    (clean_title, due_date, reminder_time)
}

fn parse_date_token(token: &str, today: NaiveDate) -> Option<NaiveDate> {
    let clean = token.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '/');
    if clean.is_empty() {
        return None;
    }

    if clean == "today" {
        return Some(today);
    }
    if clean == "tomorrow" {
        return Some(today + chrono::Duration::days(1));
    }

    // Weekdays
    let weekday = match clean {
        "monday" | "mon" => Some(Weekday::Mon),
        "tuesday" | "tue" => Some(Weekday::Tue),
        "wednesday" | "wed" => Some(Weekday::Wed),
        "thursday" | "thu" => Some(Weekday::Thu),
        "friday" | "fri" => Some(Weekday::Fri),
        "saturday" | "sat" => Some(Weekday::Sat),
        "sunday" | "sun" => Some(Weekday::Sun),
        _ => None,
    };

    if let Some(wd) = weekday {
        let mut d = today + chrono::Duration::days(1);
        while d.weekday() != wd {
            d = d + chrono::Duration::days(1);
        }
        return Some(d);
    }

    // YYYY-MM-DD
    if let Ok(date) = NaiveDate::parse_from_str(clean, "%Y-%m-%d") {
        return Some(date);
    }

    // YYYY/MM/DD
    if let Ok(date) = NaiveDate::parse_from_str(clean, "%Y/%m/%d") {
        return Some(date);
    }

    // MM-DD or MM/DD
    if let Some(date) = parse_mm_dd(clean, today.year()) {
        return Some(date);
    }

    None
}

fn parse_mm_dd(token: &str, year: i32) -> Option<NaiveDate> {
    let parts: Vec<&str> = token.split(|c| c == '-' || c == '/').collect();
    if parts.len() == 2 {
        let mm = parts[0].parse::<u32>().ok()?;
        let dd = parts[1].parse::<u32>().ok()?;
        NaiveDate::from_ymd_opt(year, mm, dd)
    } else {
        None
    }
}

fn parse_time_token(token: &str) -> Option<NaiveTime> {
    let clean = token.trim_matches(|c: char| !c.is_alphanumeric() && c != ':');
    if clean.is_empty() {
        return None;
    }

    // HH:MM:SS
    if let Ok(time) = NaiveTime::parse_from_str(clean, "%H:%M:%S") {
        return Some(time);
    }

    // HH:MM
    if let Ok(time) = NaiveTime::parse_from_str(clean, "%H:%M") {
        return Some(time);
    }

    // Check am/pm suffixes
    let token_lower = clean.to_lowercase();
    let is_am = token_lower.ends_with("am");
    let is_pm = token_lower.ends_with("pm");

    if is_am || is_pm {
        let time_str = &token_lower[..token_lower.len() - 2];
        if let Ok(time) = NaiveTime::parse_from_str(time_str, "%H:%M") {
            let mut hour = time.hour();
            if is_pm && hour < 12 {
                hour += 12;
            } else if is_am && hour == 12 {
                hour = 0;
            }
            return NaiveTime::from_hms_opt(hour, time.minute(), 0);
        }

        if let Ok(hour) = time_str.parse::<u32>() {
            if hour <= 12 {
                let mut h = hour;
                if is_pm && h < 12 {
                    h += 12;
                } else if is_am && h == 12 {
                    h = 0;
                }
                return NaiveTime::from_hms_opt(h, 0, 0);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_task_input() {
        let today = Local::now().date_naive();

        // 1. Natural language dates & times
        let (title, due, time) = parse_task_input("Buy groceries tomorrow at 5pm");
        assert_eq!(title, "Buy groceries");
        assert_eq!(due, Some(today + chrono::Duration::days(1)));
        assert_eq!(time, Some(NaiveTime::from_hms_opt(17, 0, 0).unwrap()));

        // 2. Exact date & time formats
        let (title2, due2, time2) = parse_task_input("Call dentist on 2026-07-01 at 14:30");
        assert_eq!(title2, "Call dentist");
        assert_eq!(due2, Some(NaiveDate::from_ymd_opt(2026, 7, 1).unwrap()));
        assert_eq!(time2, Some(NaiveTime::from_hms_opt(14, 30, 0).unwrap()));

        // 3. Weekday & time formats
        let (title3, due3, time3) = parse_task_input("Do laundry Friday 10:00");
        assert_eq!(title3, "Do laundry");
        assert!(due3.is_some());
        assert_eq!(due3.unwrap().weekday(), Weekday::Fri);
        assert_eq!(time3, Some(NaiveTime::from_hms_opt(10, 0, 0).unwrap()));

        // 4. Time only (should not set date here, handled by caller)
        let (title4, due4, time4) = parse_task_input("Meeting at 9am");
        assert_eq!(title4, "Meeting");
        assert_eq!(due4, None);
        assert_eq!(time4, Some(NaiveTime::from_hms_opt(9, 0, 0).unwrap()));

        // 5. Month/Day format
        let (title5, due5, time5) = parse_task_input("Submit report 12-25 14:00");
        assert_eq!(title5, "Submit report");
        assert_eq!(due5, Some(NaiveDate::from_ymd_opt(today.year(), 12, 25).unwrap()));
        assert_eq!(time5, Some(NaiveTime::from_hms_opt(14, 0, 0).unwrap()));

        // 6. Prepositions without valid target
        let (title6, due6, time6) = parse_task_input("Talk to boss at office");
        assert_eq!(title6, "Talk to boss at office");
        assert_eq!(due6, None);
        assert_eq!(time6, None);
    }
}
