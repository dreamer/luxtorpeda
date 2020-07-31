extern crate reqwest;
extern crate tar;
extern crate xz2;

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::io::{Error, ErrorKind};
use std::path::{Path, PathBuf};
use std::ffi::OsStr;
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;
use tar::Archive;
use xz2::read::XzDecoder;
use bzip2::read::BzDecoder;
use flate2::read::GzDecoder;
use sha1::{Sha1, Digest};
use std::collections::HashMap;
use std::process::Command;
use std::fs::File;
use std::io::Write;
use std::env;

use crate::ipc;
use crate::user_env;

fn place_cached_file(app_id: &str, file: &str) -> io::Result<PathBuf> {
    let xdg_dirs = xdg::BaseDirectories::new().unwrap();
    let path_str = format!("luxtorpeda/{}/{}", app_id, file);
    xdg_dirs.place_cache_file(path_str)
}

fn find_cached_file(app_id: &str, file: &str) -> Option<PathBuf> {
    let xdg_dirs = xdg::BaseDirectories::new().unwrap();
    let path_str = format!("luxtorpeda/{}/{}", app_id, file);
    xdg_dirs.find_cache_file(path_str)
}

fn place_config_file(app_id: &str, file: &str) -> io::Result<PathBuf> {
    let xdg_dirs = xdg::BaseDirectories::new().unwrap();
    let path_str = format!("luxtorpeda/{}/{}", app_id, file);
    xdg_dirs.place_config_file(path_str)
}

pub fn find_config_file(app_id: &str, file: &str) -> Option<PathBuf> {
    let xdg_dirs = xdg::BaseDirectories::new().unwrap();
    let path_str = format!("luxtorpeda/{}/{}", app_id, file);
    xdg_dirs.find_config_file(path_str)
}

#[derive(Serialize, Deserialize, Debug)]
pub struct CmdReplacement {
    #[serde(with = "serde_regex")]
    pub match_cmd: Regex,
    pub cmd: String,
    pub args: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct PackageMetadata {
    engine_version: String,
    commands: Vec<CmdReplacement>,
}

struct PackageInfo {
    name: String,
    url: String,
    file: String,
    cache_by_name: bool
}

pub fn get_zenity_path() -> Result<String, Error>  {
    let zenity_path = match env::var("STEAM_ZENITY") {
        Ok(s) => s,
        Err(_) => {
            return Err(Error::new(ErrorKind::Other, "Path could not be found"));
        }
    };
    
    return Ok(zenity_path);
}

pub fn read_cmd_repl_from_file<P: AsRef<Path>>(path: P) -> Result<Vec<CmdReplacement>, Error> {
    let file = fs::File::open(path)?;
    let reader = io::BufReader::new(file);
    let meta: PackageMetadata = serde_json::from_reader(reader)?;
    Ok(meta.commands)
}

pub fn get_remote_packages_hash(remote_hash_url: &String) -> Option<String> {
    let remote_hash_response = match reqwest::blocking::get(remote_hash_url) {
        Ok(s) => s,
        Err(err) => {
            println!("get_remote_packages_hash error in get: {:?}", err);
            return None;
        }
    };
    
    let remote_hash_str = match remote_hash_response.text() {
        Ok(s) => s,
        Err(err) => {
            println!("get_remote_packages_hash error in text: {:?}", err);
            return None;
        }
    };
    
    return Some(remote_hash_str);
}

pub fn generate_hash_from_file_path(file_path: &std::path::PathBuf) -> io::Result<String> {
    let json_str = fs::read_to_string(file_path)?;
    let mut hasher = Sha1::new();
    hasher.update(json_str);
    let hash_result = hasher.finalize();
    let hash_str = hex::encode(hash_result);
    return Ok(hash_str);
}

pub fn update_packages_json() -> io::Result<()> {
    let config_json_file = user_env::tool_dir().join("config.json");
    let config_json_str = fs::read_to_string(config_json_file)?;
    let config_parsed = json::parse(&config_json_str).unwrap();
    
    let should_do_update = &config_parsed["should_do_update"];
    if should_do_update != true {
        return Ok(());
    }
    
    let packages_json_file = user_env::tool_dir().join("packages.json");
    let mut should_download = true;
    let mut remote_hash_str: String = String::new();
    
    let remote_hash_url = std::format!("{0}/packages.hash", &config_parsed["host_url"]);
    match get_remote_packages_hash(&remote_hash_url) {
        Some(tmp_hash_str) => {
            remote_hash_str = tmp_hash_str;
        },
        None => {
            println!("update_packages_json in get_remote_packages_hash call. received none");
            should_download = false;
        }
    }
    
    if should_download {
        if !Path::new(&packages_json_file).exists() {
            should_download = true;
            println!("update_packages_json. packages.json does not exist");
        } else {
            let hash_str = generate_hash_from_file_path(&packages_json_file)?;
            println!("update_packages_json. found hash: {}", hash_str);
            
            println!("update_packages_json. found hash and remote hash: {0} {1}", hash_str, remote_hash_str);
            if hash_str != remote_hash_str {
                println!("update_packages_json. hash does not match. downloading");
                should_download = true;
            } else {
                should_download = false;
            }
        }
    }
    
    if should_download {
        println!("update_packages_json. downloading new packages.json");
        
        let remote_packages_url = std::format!("{0}/packages.json", &config_parsed["host_url"]);
        let mut download_complete = false;
        let local_packages_temp_path = user_env::tool_dir().join("packages-temp.json");
        
        match reqwest::blocking::get(&remote_packages_url) {
            Ok(mut response) => {
                let mut dest = fs::File::create(&local_packages_temp_path)?;
                io::copy(&mut response, &mut dest)?;
                download_complete = true;
            }
            Err(err) => {
                println!("update_packages_json. download err: {:?}", err);
            }
        }
        
        if download_complete {
            let new_hash_str = generate_hash_from_file_path(&local_packages_temp_path)?;
            if new_hash_str == remote_hash_str {
                println!("update_packages_json. new downloaded hash matches");
                fs::rename(local_packages_temp_path, packages_json_file)?;
            } else {
                println!("update_packages_json. new downloaded hash does not match");
                fs::remove_file(local_packages_temp_path)?;
            }
        }
    }
    
    Ok(())
}

fn pick_engine_choice(app_id: &str, game_info: &json::JsonValue) -> io::Result<String> {
    let mut zenity_list_command: Vec<String> = vec![
        "--list".to_string(),
        "--title=Pick the engine below".to_string(),
        "--column=Name".to_string(),
        "--hide-header".to_string()
    ];
    
    for entry in game_info["choices"].members() {
        if entry["name"].is_null() {
            return Err(Error::new(ErrorKind::Other, "missing choice info"));
        }
        zenity_list_command.push(entry["name"].to_string());
    }
    
    let zenity_path = match get_zenity_path() {
        Ok(s) => s,
        Err(_) => {
            return Err(Error::new(ErrorKind::Other, "zenity path not found"))
        }
    };
    
    let choice = Command::new(zenity_path)
        .args(&zenity_list_command)
        .output()
        .expect("failed to show choices");
                                    
    if !choice.status.success() {
        println!("show choice. dialog was rejected");
        
        let choice_file_path = place_config_file(&app_id, "engine_choice.txt")?;
        if choice_file_path.exists() {
            fs::remove_file(choice_file_path)?;
        }
        
        return Err(Error::new(ErrorKind::Other, "dialog was rejected"));
    }
    
    let choice_name = match String::from_utf8(choice.stdout) {
        Ok(s) => String::from(s.trim()),
        Err(_) => {
            return Err(Error::new(ErrorKind::Other, "Failed to parse choice name"));
        }
    };
    
    println!("engine choice: {:?}", choice_name);
    
    let choice_file_path = place_config_file(&app_id, "engine_choice.txt")?;
    let mut choice_file = File::create(choice_file_path)?;
    choice_file.write_all(choice_name.as_bytes())?;
    
    return Ok(choice_name);
}

pub fn convert_game_info_with_choice(choice_name: String, game_info: &mut json::JsonValue) -> io::Result<()> {
    let mut choice_data = HashMap::new();
    let mut download_array = json::JsonValue::new_array();
    
    if game_info["choices"].is_null() {
        return Err(Error::new(ErrorKind::Other, "choices array null"));
    }
    
    for entry in game_info["choices"].members() {
        if entry["name"].is_null() {
            return Err(Error::new(ErrorKind::Other, "missing choice info"));
        }
        choice_data.insert(
            entry["name"].to_string(),
            entry.clone()
        );
    }
    
    if !choice_data.contains_key(&choice_name) {
        return Err(Error::new(ErrorKind::Other, "choices array does not contain engine choice"));
    }
    
    for (key, value) in choice_data[&choice_name].entries() {
        if key == "name" || key == "download" {
            continue;
        }
        game_info[key] = value.clone();
    }
    
    for entry in game_info["download"].members() {
        if choice_data[&choice_name]["download"].is_null() || choice_data[&choice_name]["download"].contains(entry["name"].clone()) {
            match download_array.push(entry.clone()) {
                Ok(()) => {},
                Err(_) => {
                    return Err(Error::new(ErrorKind::Other, "Error pushing to download array"));
                }
            };
        }
    }
    
    game_info["download"] = download_array;
    game_info.remove("choices");
    
    return Ok(());
}

fn json_to_downloads(app_id: &str, game_info: &json::JsonValue) -> io::Result<Vec<PackageInfo>> {
    let mut downloads: Vec<PackageInfo> = Vec::new();

    for entry in game_info["download"].members() {
        if entry["name"].is_null() || entry["url"].is_null() || entry["file"].is_null() {
            return Err(Error::new(ErrorKind::Other, "missing download info"));
        }

        let name = entry["name"].to_string();
        let url = entry["url"].to_string();
        let file = entry["file"].to_string();
        let mut cache_by_name = false;
        
        let mut cache_dir = app_id;
        if entry["cache_by_name"] == true {
            cache_dir = &name;
            cache_by_name = true;
        }

        if find_cached_file(cache_dir, file.as_str()).is_some() {
            println!("{} found in cache (skip)", file);
            continue;
        }

        downloads.push(PackageInfo { name, url, file, cache_by_name });
    }
    Ok(downloads)
}

pub fn download_all(app_id: String) -> io::Result<()> {
    update_packages_json().unwrap();
    
    let mut game_info = get_game_info(app_id.as_str())
        .ok_or_else(|| Error::new(ErrorKind::Other, "missing info about this game"))?;
        
    if !game_info["choices"].is_null() {
        println!("showing engine choices");
        
        let engine_choice = match pick_engine_choice(app_id.as_str(), &game_info) {
            Ok(s) => s,
            Err(err) => {
                return Err(err);
            }
        };

        match convert_game_info_with_choice(engine_choice, &mut game_info) {
            Ok(()) => {
                println!("engine choice complete");
            },
            Err(err) => {
                return Err(err);
            }
        };
    }

    if game_info["download"].is_null() {
        println!("skipping downloads (no urls defined for this package)");
        return Ok(());
    }

    let downloads = json_to_downloads(app_id.as_str(), &game_info)?;

    if downloads.is_empty() {
        return Ok(());
    }
    
    let mut dialog_message = String::new();
    
    if !game_info["information"].is_null() && game_info["information"]["non_free"] == true {
        dialog_message = std::format!("This engine uses a non-free engine ({0}). Are you sure you want to continue?", game_info["information"]["license"]);
    }
    else if !game_info["information"].is_null() && game_info["information"]["closed_source"] == true {
        dialog_message = "This engine uses assets from the closed source release. Are you sure you want to continue?".to_string();
    }
    
    if !dialog_message.is_empty() {
        let zenity_path = match get_zenity_path() {
            Ok(s) => s,
            Err(_) => {
                return Err(Error::new(ErrorKind::Other, "zenity path not found"))
            }
        };
        
        let zenity_command: Vec<String> = vec![
            "--question".to_string(),
             std::format!("--text={}", dialog_message).to_string(),
            "--title=License Warning".to_string()
        ];
        
        let choice = Command::new(zenity_path)
            .args(&zenity_command)
            .status()
            .expect("failed to show license warning");

        if !choice.success() {
            println!("show license warning. dialog was rejected");
            return Err(Error::new(ErrorKind::Other, "dialog was rejected"));
        }
    }

    let (tx, rx): (Sender<ipc::StatusMsg>, Receiver<ipc::StatusMsg>) = mpsc::channel();

    let app = app_id.clone();
    let status_relay = thread::spawn(move || {
        ipc::status_relay(rx, app);
    });

    let mut err = Ok(());
    let n = downloads.len() as i32;
    for (i, info) in downloads.iter().enumerate() {
        // update status
        //
        match tx.send(ipc::StatusMsg::Status(i as i32, n, info.name.clone())) {
            Ok(()) => {}
            Err(e) => {
                print!("err: {}", e);
            }
        }
        err = download(app_id.as_str(), info);
    }

    // stop relay thread
    //
    match tx.send(ipc::StatusMsg::Done) {
        Ok(()) => {}
        Err(e) => {
            print!("err: {}", e);
        }
    };

    status_relay.join().expect("status relay thread panicked");

    err
}

fn download(app_id: &str, info: &PackageInfo) -> io::Result<()> {
    let target = info.url.clone() + &info.file;

    // TODO handle 404 and other errors
    //
    
    let mut cache_dir = app_id;
    if info.cache_by_name == true {
        cache_dir = &info.name;
    }
    
    match reqwest::blocking::get(target.as_str()) {
        Ok(mut response) => {
            let dest_file = place_cached_file(&cache_dir, &info.file)?;
            let mut dest = fs::File::create(dest_file)?;
            io::copy(&mut response, &mut dest)?;
            Ok(())
        }
        Err(err) => {
            println!("download err: {:?}", err);
            Err(Error::new(ErrorKind::Other, "download error"))
        }
    }
}

fn unpack_tarball(tarball: &PathBuf, game_info: &json::JsonValue, name: &str) -> io::Result<()> {
    let package_name = tarball
        .file_name()
        .and_then(|x| x.to_str())
        .and_then(|x| x.split('.').next())
        .ok_or_else(|| Error::new(ErrorKind::Other, "package has no name?"))?;

    let transform = |path: &PathBuf| -> PathBuf {
        match path.as_path().to_str() {
            Some("manifest.json") => PathBuf::from(format!("manifests.lux/{}.json", &package_name)),
            _ => PathBuf::from(path.strip_prefix("dist").unwrap_or(&path)),
        }
    };

    eprintln!("installing: {}", package_name);
    
    let mut extract_location: String = String::new();
    let mut strip_prefix: String = String::new();
    let mut decode_as_zip = false;
    
    if !&game_info["download_config"].is_null() && !&game_info["download_config"][&name.to_string()].is_null() {
        let file_download_config = &game_info["download_config"][&name.to_string()];
        if !file_download_config["extract_location"].is_null() {
            extract_location = file_download_config["extract_location"].to_string();
            println!("install changing extract location with config {}", extract_location);
        }
        if !file_download_config["strip_prefix"].is_null() {
            strip_prefix = file_download_config["strip_prefix"].to_string();
            println!("install changing prefix with config {}", strip_prefix);
        }
        if !file_download_config["decode_as_zip"].is_null() && file_download_config["decode_as_zip"] == true {
            decode_as_zip = true;
            println!("install changing decoder to zip");
        }
    }

    let file = fs::File::open(&tarball)?;
    
    if decode_as_zip {
        let mut archive = zip::ZipArchive::new(file).unwrap();
        for i in 0..archive.len() {
            let mut file = archive.by_index(i).unwrap();
            
            if file.is_dir() {
                continue;
            }
            
            let mut new_path = PathBuf::from(file.name());
            
            if !strip_prefix.is_empty() {
                new_path = new_path.strip_prefix(&strip_prefix).unwrap().to_path_buf();
            }
            
            if !extract_location.is_empty() {
                new_path = Path::new(&extract_location).join(new_path);
            }
            
            println!("install: {:?}", &new_path);
            
            if new_path.parent().is_some() {
                fs::create_dir_all(new_path.parent().unwrap())?;
            }
            
            let _ = fs::remove_file(&new_path);
            let mut outfile = fs::File::create(&new_path).unwrap();
            io::copy(&mut file, &mut outfile).unwrap();
        }
    } else {
        let file_extension = Path::new(&tarball).extension().and_then(OsStr::to_str).unwrap();
        let decoder: Box<dyn std::io::Read>;
        
        if file_extension == "bz2" {
            decoder = Box::new(BzDecoder::new(file));
        } else if file_extension == "gz" {
            decoder = Box::new(GzDecoder::new(file));
        } else {
            decoder = Box::new(XzDecoder::new(file));
        }
        
        let mut archive = Archive::new(decoder);

        for entry in archive.entries()? {
            let mut file = entry?;
            let old_path = PathBuf::from(file.header().path()?);
            let mut new_path = transform(&old_path);
            if new_path.to_str().map_or(false, |x| x.is_empty()) {
                continue;
            }
            
            if !strip_prefix.is_empty() {
                new_path = new_path.strip_prefix(&strip_prefix).unwrap().to_path_buf();
            }
            
            if !extract_location.is_empty() {
                new_path = Path::new(&extract_location).join(new_path);
            }
            
            println!("install: {:?}", &new_path);
            if new_path.parent().is_some() {
                fs::create_dir_all(new_path.parent().unwrap())?;
            }
            let _ = fs::remove_file(&new_path);
            file.unpack(&new_path)?;
        }
    }

    Ok(())
}

fn copy_only(path: &PathBuf) -> io::Result<()> {
    let package_name = path
        .file_name()
        .and_then(|x| x.to_str())
        .ok_or_else(|| Error::new(ErrorKind::Other, "package has no name?"))?;

    eprintln!("copying: {}", package_name);
    fs::copy(path, package_name)?;

    Ok(())
}

pub fn is_setup_complete(setup_info: &json::JsonValue) -> bool {
    let setup_complete = Path::new(&setup_info["complete_path"].to_string()).exists();
    return setup_complete;
}

pub fn install(game_info: &json::JsonValue) -> io::Result<()> {
    let app_id = user_env::steam_app_id();

    let packages: std::slice::Iter<'_, json::JsonValue> = game_info["download"]
        .members();
        
    let mut setup_complete = false;
    if !game_info["setup"].is_null() {
        setup_complete = is_setup_complete(&game_info["setup"]);
    }

    for file_info in packages {
        let file = file_info["file"].as_str()
            .ok_or_else(|| Error::new(ErrorKind::Other, "missing info about this game"))?;
            
        let name = file_info["name"].to_string();
        let mut cache_dir = &app_id;
        if file_info["cache_by_name"] == true {
            cache_dir = &name;
        }
        
        if setup_complete && !&game_info["download_config"].is_null() && !&game_info["download_config"][&name.to_string()].is_null() && !&game_info["download_config"][&name.to_string()]["setup"].is_null() && game_info["download_config"][&name.to_string()]["setup"] == true {
            continue;
        }
        
        match find_cached_file(&cache_dir, &file) {
            Some(path) => {
                if file_info["copy_only"] == true {
                    copy_only(&path)?;
                }
                else if
                    !&game_info["download_config"].is_null() &&
                    !&game_info["download_config"][&name.to_string()].is_null() &&
                    !&game_info["download_config"][&name.to_string()]["copy_only"].is_null() &&
                    game_info["download_config"][&name.to_string()]["copy_only"] == true {
                    copy_only(&path)?;
                }
                else {
                    unpack_tarball(&path, &game_info, &name)?;
                }
            }
            None => {
                return Err(Error::new(ErrorKind::Other, "package file not found"));
            }
        }
    }
    Ok(())
}

pub fn get_game_info(app_id: &str) -> Option<json::JsonValue> {
    let packages_json_file = user_env::tool_dir().join("packages.json");
    let json_str = match fs::read_to_string(packages_json_file) {
        Ok(s) => s,
        Err(err) => {
            println!("read err: {:?}", err);
            return None;
        }
    };
    let parsed = match json::parse(&json_str) {
        Ok(j) => j,
        Err(err) => {
            println!("parsing err: {:?}", err);
            return None;
        }
    };
    let game_info = parsed[app_id].clone();
    if game_info.is_null() {
        if !parsed["default"].is_null() {
            println!("game info using default");
            return Some(parsed["default"].clone());
        }
        else {
            None
        }
    } else {
        Some(game_info)
    }
}
