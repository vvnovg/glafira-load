//! Профили нагрузки: проверка при сохранении, расчёт, перевод ошибок модели в `AppError`.

use glafira_load_model::profile::{Computed, ProfileError, ProfileFile, compute};

use crate::error::{AppError, AppResult, ErrorCode};
use crate::project::manifest::ProjectDir;

/// `ProfileError` не сериализуется — переводим в ошибку с ключом и параметрами.
pub fn map_err(e: ProfileError) -> AppError {
    match e {
        ProfileError::UnknownProfile(name) => app_err!(
            ErrorCode::NotFound,
            "errors.profile.notFound",
            "Profile {{name}} not found",
            name = name
        ),
        ProfileError::InvalidValue { field, value } => app_err!(
            ErrorCode::InvalidInput,
            "errors.profile.invalidValue",
            "Invalid value in the profile: {{field}} = {{value}}",
            field = field,
            value = value
        ),
        ProfileError::TooManySteps { phase, max } => app_err!(
            ErrorCode::InvalidInput,
            "errors.profile.tooManySteps",
            "Phase {{phase}} has too many steps (limit {{max}})",
            // Для пользователя фазы нумеруются с 1.
            phase = phase + 1,
            max = max
        ),
    }
}

pub fn calculate(d: &ProjectDir, name: &str) -> AppResult<Computed> {
    compute(&d.profile()?, name, &d.registry()?, &d.prod_load()?).map_err(map_err)
}

/// Сохранение: хотя бы один профиль, и каждый считается без ошибок (коэффициенты и фазы).
pub fn save(d: &ProjectDir, file: &ProfileFile) -> AppResult<()> {
    if file.profiles.is_empty() {
        return Err(app_err!(
            ErrorCode::InvalidInput,
            "errors.profile.noProfiles",
            "Add at least one profile"
        ));
    }
    let registry = d.registry()?;
    let prod = d.prod_load()?;
    for name in file.profiles.keys() {
        compute(file, name, &registry, &prod).map_err(map_err)?;
    }
    d.save_profile(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load::template::tests::project_with_ops;
    use glafira_load_model::profile::Phase;

    #[test]
    fn profile_errors_carry_field_and_value() {
        let (_tmp, d) = project_with_ops();
        let mut f = d.profile().unwrap();
        f.coefficients.k_env = -1.0;
        let e = save(&d, &f).unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidInput);
        assert_eq!(e.params["field"], "coefficients.k_env");
        assert_eq!(e.params["value"], "-1");

        let mut f = d.profile().unwrap();
        f.profiles.get_mut("stress").unwrap().phases = vec![Phase::Steps {
            step_pct: 0.001,
            step_minutes: 1.0,
            max_pct: 300.0,
        }];
        let e = save(&d, &f).unwrap_err();
        assert_eq!(e.key, "errors.profile.tooManySteps");
        assert_eq!(e.params["phase"], "1");

        f.profiles.get_mut("stress").unwrap().phases = vec![Phase::Steady { minutes: f64::NAN }];
        assert_eq!(save(&d, &f).unwrap_err().key, "errors.profile.invalidValue");
        assert!(
            !d.file("load/profile.yaml").exists(),
            "ошибочный профиль не записывается"
        );

        f.profiles.clear();
        assert_eq!(save(&d, &f).unwrap_err().key, "errors.profile.noProfiles");
    }
}
