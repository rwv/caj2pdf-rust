// SPDX-License-Identifier: MIT

//! The independently measured PointsM / Link / LineStyle4 / ShowType1 profile.
//! Unknown appearances fail native composition; damaged packages retain the
//! existing located warning. Targets are decoded as data and never followed.

use super::*;
use crate::Limits;
use std::mem::size_of;

const NOTES: &[&str] = &["Package", "Note-Package", "NoteItems"];
const LINK: &[&str] = &["Package", "Note-Package", "NoteItems", "Item"];
const MAX_LINKS: usize = 8192;
const MAX_COORDINATE: i32 = 1_440_000;

#[derive(Debug)]
pub(crate) struct Link {
    pub page: u32,
    ordinal: u32,
    /// Hundredths of points, top-left origin: left, top, right, bottom.
    pub rect: [i32; 4],
    pub rgb: [u8; 3],
    pub uri: String,
}

impl<S: RangedSource, C: Cancellation> Hnc8Reader<'_, S, C> {
    pub(crate) fn application_info_with_links(
        &mut self,
    ) -> Result<(ApplicationInfoReport, Vec<Link>)> {
        let read: Result<_> = (|| {
            let Some((xml, loc)) = self.application_info_xml()? else {
                return Ok((ApplicationInfoReport::default(), Vec::new()));
            };
            // Validate the entire XML first, so a damaged tail remains a warning
            // even if it contains an unfamiliar note before the defect.
            let info = scan(&xml, loc)?;
            let links = parse(
                &xml,
                &info,
                self.header.page_count,
                self.limits,
                self.cancellation,
                loc,
            )?;
            Ok((
                ApplicationInfoReport {
                    info: Some(info),
                    status: ApplicationInfoStatus::Read,
                },
                links,
            ))
        })();
        match read {
            Ok(value) => Ok(value),
            Err(error)
                if matches!(
                    error.kind,
                    ErrorKind::Cancelled | ErrorKind::UnsupportedFormat
                ) =>
            {
                Err(error)
            }
            Err(error) => Ok((
                ApplicationInfoReport {
                    info: None,
                    status: ApplicationInfoStatus::Ignored(ApplicationInfoDefect::from(&error)),
                },
                Vec::new(),
            )),
        }
    }
}

/// Fixed-size attribute directory; no XML DOM or per-element allocation.
struct Attributes<'a> {
    entries: [(&'a str, &'a str); 16],
    len: usize,
}

impl<'a> Attributes<'a> {
    fn read(tag: &'a str, allowed: &[&str], loc: Location) -> Result<Self> {
        let (_, _, _) =
            start_tag(tag).ok_or(loc.malformed("application-info link: malformed attributes"))?;
        let mut rest = &tag[1..];
        let name_end = rest.bytes().take_while(|b| is_name_byte(*b)).count();
        rest = &rest[name_end..];
        let mut result = Self {
            entries: [("", ""); 16],
            len: 0,
        };
        loop {
            rest = rest.trim_start_matches([' ', '\t', '\r', '\n']);
            if rest.starts_with('>') || rest.starts_with("/>") {
                break;
            }
            let end = rest.bytes().take_while(|b| is_name_byte(*b)).count();
            let name = &rest[..end];
            rest = rest[end..].trim_start_matches([' ', '\t', '\r', '\n']);
            rest = rest[1..].trim_start_matches([' ', '\t', '\r', '\n']); // validated '='
            let quote = rest.as_bytes()[0] as char;
            rest = &rest[1..];
            let end = rest
                .find(quote)
                .expect("start_tag checked the quoted value");
            let value = &rest[..end];
            rest = &rest[end + 1..];
            if result.entries[..result.len]
                .iter()
                .any(|(key, _)| *key == name)
            {
                return Err(loc.malformed("application-info link: duplicate attribute"));
            }
            if !allowed.contains(&name) {
                return Err(loc.unsupported("application-info link attribute"));
            }
            if result.len == result.entries.len() {
                return Err(loc.unsupported("application-info link attribute count"));
            }
            result.entries[result.len] = (name, value);
            result.len += 1;
        }
        Ok(result)
    }

    fn value(&self, name: &str, limits: &Limits, loc: Location) -> Result<String> {
        let raw = self.entries[..self.len]
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| *value)
            .ok_or(loc.malformed("application-info link: missing attribute"))?;
        let max = len_u64(MAX_APPLICATION_INFO_FIELD_BYTES).min(limits.max_allocation_bytes);
        if len_u64(raw.len()) > max {
            return Err(loc.limit(
                "application-info link attribute bytes",
                max,
                len_u64(raw.len()),
            ));
        }
        let mut value = String::new();
        value.try_reserve_exact(raw.len()).map_err(|_| {
            loc.limit(
                "application-info link attribute bytes",
                max,
                len_u64(raw.len()),
            )
        })?;
        decode_text(raw, &mut value, loc)?;
        Ok(value)
    }

    fn require(&self, name: &str, expected: &str, limits: &Limits, loc: Location) -> Result<()> {
        if self.value(name, limits, loc)? != expected {
            return Err(loc.unsupported("application-info link appearance profile"));
        }
        Ok(())
    }
}

fn parse<C: Cancellation>(
    xml: &[u8],
    info: &ApplicationInfo,
    pages: u32,
    limits: &Limits,
    cancellation: &C,
    loc: Location,
) -> Result<Vec<Link>> {
    let count = info.note_count as usize;
    if count > MAX_LINKS {
        return Err(loc.limit("application-info links", MAX_LINKS as u64, len_u64(count)));
    }
    // The XML, exact link directory and aggregate URI bytes share this budget.
    // Each URI's encoded source is a disjoint slice of the bounded XML, and
    // entity decoding cannot expand it. A second XML-sized allowance covers it.
    // The fixed field allowance also covers both metadata passes, their active
    // capture strings and the current decoded attribute.
    let planned = len_u64(xml.len()) * 2
        + len_u64(count * size_of::<Link>())
        + 6 * len_u64(MAX_APPLICATION_INFO_FIELD_BYTES);
    if planned > limits.max_allocation_bytes {
        return Err(loc.limit(
            "application-info link allocation bytes",
            limits.max_allocation_bytes,
            planned,
        ));
    }
    let mut links = Vec::new();
    reserve_exact(
        &mut links,
        count,
        loc.limit(
            "application-info link allocation bytes",
            limits.max_allocation_bytes,
            planned,
        ),
    )?;
    let mut profile = false;
    let mut pending: Option<Link> = None;
    let mut has_rect = false;
    let mut has_target = false;
    scan_with(xml, loc, &mut |event| {
        if cancellation.is_cancelled() {
            return Err(loc.cancelled());
        }
        match event {
            Event::Start {
                parents,
                name: "NoteItems",
                tag,
                ..
            } if parents == &NOTES[..2] => {
                let attrs = Attributes::read(
                    tag,
                    &[
                        "Author",
                        "TextEncode1",
                        "ReadOnly",
                        "Version",
                        "Unit",
                        "Embedded",
                    ],
                    loc,
                )?;
                // Empty note lists require no appearance profile.
                profile = attrs
                    .value("Version", limits, loc)
                    .is_ok_and(|s| s == "1.0")
                    && attrs
                        .value("Unit", limits, loc)
                        .is_ok_and(|s| s == "PointsM");
            }
            Event::Start {
                parents,
                name: "Item",
                empty,
                tag,
            } if parents == NOTES => {
                if !profile {
                    return Err(loc.unsupported("application-info link units or version"));
                }
                let attrs = Attributes::read(
                    tag,
                    &["acp", "Type", "Page", "Color", "LineStyle", "ShowType"],
                    loc,
                )?;
                for (name, value) in [("Type", "Link"), ("LineStyle", "4"), ("ShowType", "1")] {
                    attrs.require(name, value, limits, loc)?;
                }
                if empty {
                    return Err(
                        loc.malformed("application-info link: missing rectangle and target")
                    );
                }
                let page = attrs
                    .value("Page", limits, loc)?
                    .parse::<u32>()
                    .map_err(|_| loc.malformed("application-info link: invalid page"))?;
                if page == 0 || page > pages {
                    return Err(loc.malformed("application-info link: page outside document"));
                }
                let color = attrs.value("Color", limits, loc)?;
                if color.len() != 8 || !color.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err(loc.malformed("application-info link: invalid color"));
                }
                let color = u32::from_str_radix(&color, 16).expect("checked hexadecimal color");
                if color >> 24 != 0 {
                    return Err(loc.unsupported("application-info link color profile"));
                }
                pending = Some(Link {
                    page,
                    ordinal: links.len() as u32,
                    rect: [0; 4],
                    rgb: [color as u8, (color >> 8) as u8, (color >> 16) as u8],
                    uri: String::new(),
                });
                has_rect = false;
                has_target = false;
            }
            Event::Start {
                parents,
                name,
                empty,
                tag,
            } if parents.starts_with(LINK) => {
                if parents != LINK || !empty {
                    return Err(loc.unsupported("application-info link child structure"));
                }
                let link = pending
                    .as_mut()
                    .ok_or(loc.malformed("application-info link: orphan child"))?;
                match name {
                    "RC" => {
                        if has_rect {
                            return Err(loc.malformed("application-info link: duplicate rectangle"));
                        }
                        let attrs = Attributes::read(tag, &["l", "t", "r", "b"], loc)?;
                        for (index, key) in ["l", "t", "r", "b"].iter().enumerate() {
                            let coordinate =
                                attrs.value(key, limits, loc)?.parse::<i32>().map_err(|_| {
                                    loc.malformed("application-info link: invalid coordinate")
                                })?;
                            if !(-MAX_COORDINATE..=MAX_COORDINATE).contains(&coordinate) {
                                return Err(
                                    loc.unsupported("application-info link coordinate range")
                                );
                            }
                            link.rect[index] = coordinate;
                        }
                        if link.rect[0] >= link.rect[2] || link.rect[1] >= link.rect[3] {
                            return Err(
                                loc.malformed("application-info link: empty or inverted rectangle")
                            );
                        }
                        has_rect = true;
                    }
                    "Item" => {
                        if has_target {
                            return Err(loc.malformed("application-info link: duplicate target"));
                        }
                        let attrs = Attributes::read(tag, &["Type", "Aim", "ID"], loc)?;
                        attrs.require("Type", "UrlLink", limits, loc)?;
                        let uri = attrs.value("Aim", limits, loc)?;
                        if !uri.starts_with("http://") && !uri.starts_with("https://") {
                            return Err(loc.unsupported("application-info link URI scheme"));
                        }
                        if !uri.is_ascii() || uri.bytes().any(|b| b <= b' ' || b == 127) {
                            return Err(loc.unsupported("application-info link URI encoding"));
                        }
                        link.uri = uri;
                        has_target = true;
                    }
                    _ => return Err(loc.unsupported("application-info link child")),
                }
            }
            Event::Start { parents, .. } if parents == NOTES => {
                return Err(loc.unsupported("application-info note element"));
            }
            Event::End { path } if path == LINK => {
                if !has_rect || !has_target {
                    return Err(loc.malformed("application-info link: missing rectangle or target"));
                }
                links.push(
                    pending
                        .take()
                        .ok_or(loc.malformed("application-info link: missing item"))?,
                );
            }
            Event::Text { path, text }
                if path.starts_with(NOTES) && !text.bytes().all(is_space) =>
            {
                return Err(loc.unsupported("application-info link text content"));
            }
            _ => {}
        }
        Ok(())
    })?;
    // No sorting allocation; source order remains explicit for overlapping marks.
    links.sort_unstable_by_key(|link| (link.page, link.ordinal));
    Ok(links)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::CancelAfter;

    const ITEM: &str = "<Item Type='Link' Page='2' Color='000000FF' LineStyle='4' ShowType='1'><RC l='2500' t='2500' r='15500' b='4500'/><Item Type='UrlLink' Aim='https://example.invalid/a?x=1&amp;y=2' ID='0'/></Item>";
    fn package(items: &str) -> String {
        format!(
            "<Package><Note-Package><NoteItems Unit='PointsM' Version='1.0'>{items}</NoteItems></Note-Package></Package>"
        )
    }
    fn loc() -> Location {
        Location {
            variant: Some(Variant::C8),
            offset: 123,
            page: None,
            image: None,
        }
    }
    fn read(xml: &str, limits: &Limits, cancel: &CancelAfter) -> Result<Vec<Link>> {
        let info = scan(xml.as_bytes(), loc())?;
        parse(xml.as_bytes(), &info, 2, limits, cancel, loc())
    }

    #[test]
    fn measured_links_preserve_page_order_rectangles_color_and_entities() {
        let xml = package(&format!(
            "{ITEM}{}",
            ITEM.replace("Page='2'", "Page='1'")
                .replace("000000FF", "00808080")
        ));
        let links = read(&xml, &Limits::default(), &CancelAfter::never()).unwrap();
        assert_eq!(links.len(), 2);
        assert_eq!(
            links.iter().map(|link| link.page).collect::<Vec<_>>(),
            [1, 2]
        );
        assert_eq!(links[0].rect, [2500, 2500, 15500, 4500]);
        assert_eq!(links[1].rgb, [255, 0, 0]);
        assert_eq!(links[0].rgb, [128; 3]);
        assert_eq!(links[0].uri, "https://example.invalid/a?x=1&y=2");
        assert!(
            read(
                "<Package><Note-Package><NoteItems/></Note-Package></Package>",
                &Limits::default(),
                &CancelAfter::never()
            )
            .unwrap()
            .is_empty()
        );
    }

    #[test]
    fn unmeasured_profiles_fail_explicitly() {
        for (old, new) in [
            ("PointsM", "Points"),
            ("1.0", "2.0"),
            ("LineStyle='4'", "LineStyle='3'"),
            ("ShowType='1'", "ShowType='0'"),
            ("Type='Link'", "Type='Note'"),
            ("Type='UrlLink'", "Type='FileLink'"),
            ("000000FF", "FF0000FF"),
            ("https://example.invalid", "file:///example.invalid"),
            (" ID='0'", " Extra='value'"),
        ] {
            let xml = package(ITEM).replace(old, new);
            let error = read(&xml, &Limits::default(), &CancelAfter::never()).unwrap_err();
            assert!(
                matches!(error.kind, ErrorKind::UnsupportedFormat),
                "{old}: {error}"
            );
            assert_eq!(error.offset, Some(123));
        }
    }

    #[test]
    fn malformed_rectangles_pages_targets_and_duplicate_attributes_are_rejected() {
        for (old, new) in [
            ("Page='2'", "Page='0'"),
            ("Page='2'", "Page='3'"),
            ("Page='2'", "Page='2' Page='1'"),
            ("Color='000000FF'", "Color='not-rgb'"),
            ("r='15500'", "r='2500'"),
            ("l='2500'", "l='NaN'"),
            ("l='2500'", ""),
            ("l='2500'", "l='2500' l='2501'"),
            ("&amp;", "&unknown;"),
        ] {
            let error = read(
                &package(ITEM).replace(old, new),
                &Limits::default(),
                &CancelAfter::never(),
            )
            .unwrap_err();
            assert!(matches!(error.kind, ErrorKind::Malformed), "{old}: {error}");
        }
        for body in [
            "<Item Type='Link' Page='2' Color='00000000' LineStyle='4' ShowType='1'/>",
            "<Item Type='Link' Page='2' Color='00000000' LineStyle='4' ShowType='1'><RC l='1' t='2' r='3' b='4'/></Item>",
        ] {
            assert!(matches!(
                read(&package(body), &Limits::default(), &CancelAfter::never())
                    .unwrap_err()
                    .kind,
                ErrorKind::Malformed
            ));
        }
    }

    #[test]
    fn parsing_limits_and_cancellation_do_not_return_partial_links() {
        let xml = package(ITEM);
        let limits = Limits {
            max_allocation_bytes: 100,
            ..Limits::default()
        };
        assert!(matches!(
            read(&xml, &limits, &CancelAfter::never()).unwrap_err().kind,
            ErrorKind::LimitExceeded { .. }
        ));
        let many = package(&ITEM.repeat(MAX_LINKS + 1));
        assert!(matches!(
            read(&many, &Limits::default(), &CancelAfter::never())
                .unwrap_err()
                .kind,
            ErrorKind::LimitExceeded {
                resource: "application-info links",
                ..
            }
        ));
        let long = xml.replace(
            "https://example.invalid/a?x=1&amp;y=2",
            &format!(
                "https://example.invalid/{}",
                "a".repeat(MAX_APPLICATION_INFO_FIELD_BYTES)
            ),
        );
        assert!(matches!(
            read(&long, &Limits::default(), &CancelAfter::never())
                .unwrap_err()
                .kind,
            ErrorKind::LimitExceeded { .. }
        ));
        let counter = CancelAfter::never();
        read(&xml, &Limits::default(), &counter).unwrap();
        for allowed in 0..counter.queries() {
            assert!(matches!(
                read(&xml, &Limits::default(), &CancelAfter::new(allowed))
                    .unwrap_err()
                    .kind,
                ErrorKind::Cancelled
            ));
        }
    }
}
