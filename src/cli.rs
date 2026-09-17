use clap::{Args, Parser, Subcommand};
use thiserror::Error;

use crate::{
    config::AppConfig,
    domain::user::{PublicUser, User, UserRole},
    error::AppError,
    repositories::{
        audit,
        users::{self, CreateAdminOutcome, UserRepositoryError},
    },
    services::{auth, report},
    state::AppState,
};

#[derive(Debug, Parser)]
#[command(
    name = "teatro",
    version,
    about = "Teatro self-hosted game library server"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Start the HTTP server.
    Serve,

    /// Manage Teatro users.
    Users(UsersCommand),

    /// Print a full current-state report for this Teatro instance as JSON.
    Report,
}

#[derive(Debug, Args)]
struct UsersCommand {
    #[command(subcommand)]
    command: UserSubcommand,
}

#[derive(Debug, Subcommand)]
enum UserSubcommand {
    /// List all users.
    List(ListUsersArgs),

    /// Create a user.
    Create(CreateUserArgs),

    /// Create the first admin user, or reset it with --reset-password.
    CreateAdmin(CreateAdminArgs),

    /// Reset a user's password.
    ResetPassword(ResetPasswordArgs),

    /// Change a user's role.
    SetRole(SetRoleArgs),

    /// Delete a user.
    Delete(DeleteUserArgs),
}

#[derive(Debug, Args)]
struct ListUsersArgs {
    /// Print users as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct CreateUserArgs {
    /// Username to create.
    #[arg(long)]
    username: String,

    /// User role. Accepted values: admin, readonly.
    #[arg(long, value_parser = parse_user_role, default_value = "readonly")]
    role: UserRole,

    /// User password. Prefer TEATRO_USER_PASSWORD for automation.
    #[arg(long, env = "TEATRO_USER_PASSWORD", hide_env_values = true)]
    password: Option<String>,
}

#[derive(Debug, Args)]
struct CreateAdminArgs {
    /// Admin username to create.
    #[arg(long)]
    username: String,

    /// Admin password. Prefer TEATRO_ADMIN_PASSWORD for automation.
    #[arg(long, env = "TEATRO_ADMIN_PASSWORD", hide_env_values = true)]
    password: Option<String>,

    /// Reset the password if the user already exists.
    #[arg(long)]
    reset_password: bool,
}

#[derive(Debug, Args)]
struct ResetPasswordArgs {
    /// Username whose password should be reset.
    #[arg(long)]
    username: String,

    /// New password. Prefer TEATRO_USER_PASSWORD for automation.
    #[arg(long, env = "TEATRO_USER_PASSWORD", hide_env_values = true)]
    password: Option<String>,
}

#[derive(Debug, Args)]
struct SetRoleArgs {
    /// Username whose role should be changed.
    #[arg(long)]
    username: String,

    /// New role. Accepted values: admin, readonly.
    #[arg(long, value_parser = parse_user_role)]
    role: UserRole,
}

#[derive(Debug, Args)]
struct DeleteUserArgs {
    /// Username to delete.
    #[arg(long)]
    username: String,
}

#[derive(Debug, Error)]
pub enum CliError {
    #[error("password cannot be empty")]
    EmptyPassword,

    #[error("password confirmation did not match")]
    PasswordConfirmationMismatch,

    #[error("admin user was not found after creation")]
    AdminUserMissing,

    #[error("failed to read password: {0}")]
    ReadPassword(#[from] std::io::Error),
}

pub async fn run() -> Result<(), AppError> {
    let cli = Cli::parse();

    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => crate::run().await,
        Command::Users(users) => run_users(users.command).await,
        Command::Report => print_report().await,
    }
}

async fn run_users(command: UserSubcommand) -> Result<(), AppError> {
    match command {
        UserSubcommand::List(args) => list_users(args).await,
        UserSubcommand::Create(args) => create_user(args).await,
        UserSubcommand::CreateAdmin(args) => create_admin(args).await,
        UserSubcommand::ResetPassword(args) => reset_password(args).await,
        UserSubcommand::SetRole(args) => set_role(args).await,
        UserSubcommand::Delete(args) => delete_user(args).await,
    }
}

async fn print_report() -> Result<(), AppError> {
    let state = initialize_state().await?;
    let report = report::build(&state).await?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

async fn list_users(args: ListUsersArgs) -> Result<(), AppError> {
    let state = initialize_state().await?;
    let users = users::list(state.db()).await?;

    if args.json {
        let public_users = public_users(&users);
        println!("{}", serde_json::to_string_pretty(&public_users)?);
        return Ok(());
    }

    if users.is_empty() {
        println!("No users found.");
        return Ok(());
    }

    println!("{:<6} {:<32} ROLE", "ID", "USERNAME");
    for user in users {
        println!("{:<6} {:<32} {}", user.id, user.username, user.role);
    }

    Ok(())
}

async fn create_user(args: CreateUserArgs) -> Result<(), AppError> {
    let password = read_password(args.password)?;
    let password_hash = auth::hash_password(&password)?;
    let state = initialize_state().await?;

    let user = users::create(state.db(), &args.username, &password_hash, args.role).await?;
    record_user_event(&state, "users.created", &user).await?;

    println!("Created {} user '{}'", user.role, user.username);

    Ok(())
}

async fn create_admin(args: CreateAdminArgs) -> Result<(), AppError> {
    let password = read_password(args.password)?;
    let password_hash = auth::hash_password(&password)?;
    let state = initialize_state().await?;
    let outcome = users::create_admin(
        state.db(),
        &args.username,
        &password_hash,
        args.reset_password,
    )
    .await?;

    let admin = users::find_by_username(state.db(), &args.username)
        .await?
        .ok_or(CliError::AdminUserMissing)?;
    record_user_event(
        &state,
        match outcome {
            CreateAdminOutcome::Created => "users.admin_created",
            CreateAdminOutcome::PasswordReset => "users.admin_password_reset",
        },
        &admin,
    )
    .await?;

    match outcome {
        CreateAdminOutcome::Created => {
            println!("Created admin user '{}'", args.username);
        }
        CreateAdminOutcome::PasswordReset => {
            println!("Reset password for admin user '{}'", args.username);
        }
    }

    Ok(())
}

async fn reset_password(args: ResetPasswordArgs) -> Result<(), AppError> {
    let password = read_password(args.password)?;
    let password_hash = auth::hash_password(&password)?;
    let state = initialize_state().await?;
    let user = find_required_user(&state, &args.username).await?;
    let user = users::update_password(state.db(), user.id, &password_hash).await?;

    record_user_event(&state, "users.password_reset", &user).await?;
    println!("Reset password for user '{}'", user.username);

    Ok(())
}

async fn set_role(args: SetRoleArgs) -> Result<(), AppError> {
    let state = initialize_state().await?;
    let user = find_required_user(&state, &args.username).await?;
    let user = users::set_role(state.db(), user.id, args.role).await?;

    record_user_event(&state, "users.role_changed", &user).await?;
    println!("Set role for user '{}' to {}", user.username, user.role);

    Ok(())
}

async fn delete_user(args: DeleteUserArgs) -> Result<(), AppError> {
    let state = initialize_state().await?;
    let user = find_required_user(&state, &args.username).await?;
    let user = users::delete(state.db(), user.id).await?;

    record_user_event(&state, "users.deleted", &user).await?;
    println!("Deleted user '{}'", user.username);

    Ok(())
}

async fn initialize_state() -> Result<AppState, AppError> {
    let config = AppConfig::from_env()?;
    AppState::initialize_for_cli(config).await
}

async fn find_required_user(state: &AppState, username: &str) -> Result<User, AppError> {
    users::find_by_username(state.db(), username)
        .await?
        .ok_or_else(|| UserRepositoryError::NotFound.into())
}

async fn record_user_event(
    state: &AppState,
    action: &'static str,
    user: &User,
) -> Result<(), AppError> {
    let metadata_json = serde_json::json!({
        "username": user.username,
        "role": user.role.as_str(),
    })
    .to_string();

    audit::record(
        state.db(),
        audit::AuditEvent {
            actor_user_id: None,
            action,
            entity_type: Some("user"),
            entity_id: Some(user.id),
            metadata_json: Some(&metadata_json),
        },
    )
    .await?;

    Ok(())
}

fn public_users(users: &[User]) -> Vec<PublicUser> {
    users.iter().map(User::public).collect()
}

fn read_password(password_arg: Option<String>) -> Result<String, CliError> {
    match password_arg {
        Some(password) => validate_password(password),
        None => {
            let password = rpassword::prompt_password("Password: ")?;
            let confirmation = rpassword::prompt_password("Confirm password: ")?;

            if password != confirmation {
                return Err(CliError::PasswordConfirmationMismatch);
            }

            validate_password(password)
        }
    }
}

fn validate_password(password: String) -> Result<String, CliError> {
    if password.is_empty() {
        return Err(CliError::EmptyPassword);
    }

    Ok(password)
}

fn parse_user_role(value: &str) -> Result<UserRole, String> {
    value.parse().map_err(|error| format!("{error}"))
}
