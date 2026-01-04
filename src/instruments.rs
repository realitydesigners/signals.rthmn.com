use lazy_static::lazy_static;
use std::collections::HashMap;
use std::sync::RwLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetClass {
    Stocks,
    Forex,
    Crypto,
}

fn get_asset_class(pair: &str) -> AssetClass {
    if pair == "XAUUSD" || pair == "XAGUSD" {
        return AssetClass::Forex;
    }
    
    if pair.ends_with("USD") && pair.len() >= 6 {
        let crypto_prefixes = ["ADA", "APT", "ASM", "BIGTIME", "BTC", "CLV", "ETH", "FET",
            "FIDA", "JTO", "LTC", "MEW", "PLU", "RARI", "SAND", "SEAM",
            "SOL", "TAO", "TOKEN", "UNI", "USDC", "USDG", "USDT", "XLM",
            "XMR", "XRP", "ZEC"];
        for prefix in &crypto_prefixes {
            if pair.len() >= prefix.len() && pair.starts_with(prefix) {
                return AssetClass::Crypto;
            }
        }
    }
    
    if pair.len() == 6 && pair.chars().all(|c| c.is_ascii_alphabetic()) {
        let forex_quotes = ["USD", "JPY", "EUR", "GBP", "AUD", "CAD", "CHF", "NZD"];
        if let Some(quote) = pair.get(3..) {
            if forex_quotes.contains(&quote) {
                return AssetClass::Forex;
            }
        }
    }
    
    AssetClass::Stocks
}

fn calculate_digits_from_point(point: f64) -> u8 {
    if point >= 1.0 {
        0
    } else {
        let mut p = point;
        let mut digits = 0;
        while p < 1.0 && digits < 10 {
            p *= 10.0;
            digits += 1;
        }
        digits
    }
}

fn calculate_point_from_price(price: f64, asset_class: AssetClass) -> f64 {
    let abs_price = price.abs();
    
    match asset_class {
        AssetClass::Forex => {
            if abs_price < 10.0 {
                0.00001
            } else {
                0.001
            }
        }
        AssetClass::Crypto => {
            if abs_price >= 10000.0 {
                10.0
            } else if abs_price >= 1000.0 {
                1.0
            } else if abs_price >= 100.0 {
                0.1
            } else if abs_price >= 10.0 {
                0.01
            } else if abs_price >= 1.0 {
                0.001
            } else if abs_price >= 0.1 {
                0.0001
            } else if abs_price >= 0.01 {
                0.00001
            } else {
                0.000001
            }
        }
        AssetClass::Stocks => {
            if abs_price >= 1000.0 {
                1.0
            } else {
                0.01
            }
        }
    }
}

lazy_static! {
    static ref CONFIG_CACHE: RwLock<HashMap<String, (f64, u8)>> = 
        RwLock::new(HashMap::with_capacity(2000));
    static ref PRICE_CACHE: RwLock<HashMap<String, f64>> = 
        RwLock::new(HashMap::with_capacity(2000));
}

pub fn update_instrument_price(pair: &str, price: f64) -> bool {
    let asset_class = get_asset_class(pair);
    
    let should_update = {
        let price_cache = PRICE_CACHE.read().unwrap();
        !price_cache.contains_key(pair)
    };
    
    if should_update {
        let point = calculate_point_from_price(price, asset_class);
        let digits = calculate_digits_from_point(point);
        
        {
            let mut price_cache = PRICE_CACHE.write().unwrap();
            price_cache.insert(pair.to_string(), price);
        }
        
        {
            let mut config_cache = CONFIG_CACHE.write().unwrap();
            config_cache.insert(pair.to_string(), (point, digits));
        }
        
        true
    } else {
        false
    }
}

pub fn get_instrument_config(pair: &str) -> (f64, u8) {
    {
        let cache = CONFIG_CACHE.read().unwrap();
        if let Some(&config) = cache.get(pair) {
            return config;
        }
    }
    
    let asset_class = get_asset_class(pair);
    let point = match asset_class {
        AssetClass::Forex => {
            if pair.contains("JPY") {
                0.001
            } else {
                0.00001
            }
        }
        AssetClass::Crypto => {
            if pair == "BTCUSD" || pair == "YFIUSD" {
                10.0
            } else if pair == "MKRUSD" {
                1.0
            } else {
                0.1
            }
        }
        AssetClass::Stocks => 0.01,
    };
    
    let digits = calculate_digits_from_point(point);
    let config = (point, digits);
    
    {
        let mut cache = CONFIG_CACHE.write().unwrap();
        cache.insert(pair.to_string(), config);
    }
    
    config
}
