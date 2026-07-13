pub mod capability;
pub mod content;
pub mod css;
pub mod dash;
pub mod display_list;
pub mod fonts;
pub mod gpu;
pub mod html;
pub mod js;
pub mod js_bridge;
pub mod layout;
pub mod media_decode;
pub mod p2p;
pub mod raster;
pub mod stub;
pub mod text;
pub mod wasm;

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
