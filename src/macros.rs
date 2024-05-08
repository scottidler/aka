#[macro_export]
macro_rules! vos {
    // Match against any number of string literals separated by commas
    ( $( $x:expr ),* $(,)? ) => {
        {
            let mut temp_vec = Vec::new();
            $(
                // Convert each string literal into a String and push it to the vector
                temp_vec.push($x.to_string());
            )*
            temp_vec
        }
    };
}
