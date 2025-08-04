use regex::Regex;

use crate::entity::decoder::ALIASES;

/*
Some of the code is from dotabuff/manta
https://github.com/dotabuff/manta/blob/c5131a657683ffbf1b114152ea20432b682eb863/field_type.go

The MIT License (MIT)

Copyright (c) 2016 Elo Entertainment LLC.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
*/

static FIELD_TYPE_REGEX: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r"([^<\[\*]+)(?:<\s(.+?)\s>)?(\*)?(?:\[(.+?)\])?").unwrap()
});

#[derive(Debug)]
pub struct FieldType {
    pub base_type: String,
    pub generic_type: Option<Box<FieldType>>,
    pub is_optional: bool,
    /// the game handles arrays and plain types differently
    /// anything not an array is marked with an array size of 0 instead of 1
    pub array_size: usize,
}

impl FieldType {
    pub fn new(field_type: &str) -> Result<Self, std::io::Error> {
        // map field type to aliases
        if let Some(alias) = ALIASES.get(field_type) {
            return Self::new(alias);
        }

        let Some(caps) = FIELD_TYPE_REGEX.captures(field_type) else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Invalid field type: {field_type}"),
            ));
        };

        let Some(base_type) = caps.get(1).map(|v| v.as_str()) else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Missing base type in field type: {field_type}"),
            ));
        };

        let generic_type = if let Some(generic) = caps.get(2).map(|v| v.as_str()) {
            Some(Box::new(Self::new(generic)?))
        } else {
            None
        };

        let is_optional = caps.get(3).map(|v| v.as_str() == "*").unwrap_or(false);
        let array_size = if let Some(size) = caps.get(4).map(|v| v.as_str()) {
            size.parse().map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("Invalid array size in field type: {field_type}"),
                )
            })?
        } else {
            0
        };

        Ok(Self {
            base_type: base_type.to_string(),
            generic_type,
            is_optional,
            array_size,
        })
    }
}
