/// The fixture's one function: doubles its input.
pub fn double(x: u32) -> u32 {
    x * 2
}

#[cfg(test)]
mod tests {
    #[test]
    fn doubles() {
        assert_eq!(super::double(3), 6);
    }
}
