use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};

pub fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default().hash_password(password.as_bytes(), &salt)?;

    Ok(hash.to_string())
}

pub fn verify_password(password: &str, password_hash: &str) -> bool {
    let Ok(parsed_hash) = PasswordHash::new(password_hash) else {
        return false;
    };

    Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::{hash_password, verify_password};

    #[test]
    fn password_hash_round_trip_works() {
        let hash = hash_password("correct horse battery staple").unwrap();

        assert!(verify_password("correct horse battery staple", &hash));
        assert!(!verify_password("wrong password", &hash));
    }

    #[test]
    fn invalid_hashes_do_not_verify() {
        assert!(!verify_password("password", "not-a-password-hash"));
    }
}
