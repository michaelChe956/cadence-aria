pub const N00: &str = "N00";
pub const N01: &str = "N01";
pub const N02: &str = "N02";
pub const N03: &str = "N03";
pub const N04: &str = "N04";
pub const N05: &str = "N05";
pub const N06: &str = "N06";
pub const N07: &str = "N07";
pub const N08: &str = "N08";
pub const N09: &str = "N09";
pub const N10: &str = "N10";
pub const N11: &str = "N11";
pub const N12: &str = "N12";
pub const N13: &str = "N13";
pub const N14: &str = "N14";
pub const N15: &str = "N15";
pub const N16: &str = "N16";
pub const N17: &str = "N17";
pub const N18: &str = "N18";
pub const N19: &str = "N19";
pub const N20: &str = "N20";
pub const N21: &str = "N21";
pub const N22: &str = "N22";
pub const N23: &str = "N23";
pub const N24: &str = "N24";
pub const N25: &str = "N25";
pub const N26: &str = "N26";
pub const N27: &str = "N27";
pub const N28: &str = "N28";

pub const X01: &str = "X01";
pub const X02: &str = "X02";
pub const X03: &str = "X03";
pub const X04: &str = "X04";
pub const X05: &str = "X05";
pub const X06: &str = "X06";
pub const X07: &str = "X07";
pub const X08: &str = "X08";
pub const X09: &str = "X09";

pub fn is_protocol_node_id(node_id: &str) -> bool {
    if let Some(number) = node_id.strip_prefix('N') {
        return matches!(number.parse::<u8>(), Ok(value) if value <= 28);
    }

    if let Some(number) = node_id.strip_prefix('X') {
        return matches!(number.parse::<u8>(), Ok(value) if (1..=9).contains(&value));
    }

    false
}
