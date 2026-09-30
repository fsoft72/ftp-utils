//! Shared definition of the message-only error types used across the
//! suite. Every such error is terminal: it carries a complete, user-facing
//! message (already including file names and other context) that a tool
//! prints once and turns into an exit code.

/// Declares a tuple-struct error wrapping a message `String`, with
/// `Debug`, `Display` (the bare message) and `std::error::Error`.
///
/// ```
/// ftp_utils_core::message_error! {
///     /// Something went wrong.
///     pub ExampleError
/// }
///
/// assert_eq!(ExampleError("boom".to_string()).to_string(), "boom");
/// ```
#[macro_export]
macro_rules! message_error {
    ($(#[$meta:meta])* $vis:vis $name:ident) => {
        $(#[$meta])*
        #[derive(Debug)]
        $vis struct $name(pub String);

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl ::std::error::Error for $name {}
    };
}
