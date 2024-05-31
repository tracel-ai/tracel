use clap::Parser;
use reqwest::blocking::Client;
use reqwest::header::{COOKIE, SET_COOKIE};
use serde::Deserialize;
use serde::Serialize;
use std::error::Error;

#[derive(Serialize, Debug)]
struct ApiKeyCreds {
    api_key: String,
}

#[derive(Deserialize, Debug)]
struct UserData {
    username: String,
}

#[derive(Parser, Debug)]
#[command(name = "Login")]
#[command(about = "Login in the Heat platform example using an API key", long_about = None)]
struct Args {
    /// The API key to use for login
    #[arg(short, long)]
    key: String,

    /// Base URL of the Heat server.
    #[arg(short, long, default_value = "http://localhost:9001")]
    url: String,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();
    let client = Client::new();

    let creds = ApiKeyCreds { api_key: args.key };

    let api_key_login_endpoint = format!("{}/login/api-key", args.url);
    let user_api_key_endpoint = format!("{}/user/api-keys", args.url);
    let user_data_endpoint = format!("{}/user/me", args.url);

    let response = client.post(&api_key_login_endpoint).form(&creds).send()?;
    println!("Request URL: {}", &api_key_login_endpoint);
    println!("Request Body: {:?}", &creds);

    if response.status().is_success() {
        // Extract the session cookie from the response headers and then retrieve the user info
        if let Some(cookie_header) = response.headers().get(SET_COOKIE) {
            let cookie_str = cookie_header.to_str()?;
            println!("Received session cookie: {}", cookie_str);
            let response = client
                .get(user_api_key_endpoint)
                .header(COOKIE, cookie_str)
                .send()?;
            if response.status().is_success() {
                println!("Successfully accessed the protected resource.");
                let user_response = client
                    .get(user_data_endpoint)
                    .header(COOKIE, cookie_str)
                    .send()?;

                if user_response.status().is_success() {
                    // Deserialize the user data.
                    let user_data: UserData = user_response.json()?;
                    println!("Username: {}", user_data.username);
                } else {
                    println!(
                        "Failed to retrieve user data. Status: {}",
                        user_response.status()
                    );
                }
            } else {
                println!("Failed to access the protected resource.");
            }
        } else {
            println!("No session cookie received.");
        }
    } else {
        println!(
            "Failed to login with API key. Status: {}",
            response.status()
        );
        println!("Response Body: {:?}", response.text()?);
    }

    Ok(())
}
