use signals_rthmn::patterns::{BOXES, STARTING_POINTS};
use std::collections::HashMap;
use std::fs::File;
use std::io::Write;

fn traverse_all_paths_recursive(
    boxes: &HashMap<i32, Vec<Vec<i32>>>,
    current_key: i32,
    current_path: Vec<i32>,
    all_paths: &mut Vec<Vec<i32>>,
) {
    let abs_key = current_key.abs();
    let patterns = boxes.get(&abs_key);

    if patterns.is_none() || patterns.unwrap().is_empty() {
        all_paths.push(current_path);
        return;
    }

    for pattern in patterns.unwrap() {
        let adjusted: Vec<i32> = if current_key > 0 {
            pattern.clone()
        } else {
            pattern.iter().map(|&v| -v).collect()
        };

        let mut full_path = current_path.clone();
        full_path.extend(&adjusted);
        let last_value = *adjusted.last().unwrap();

        if last_value.abs() == current_key.abs() {
            all_paths.push(full_path);
            continue;
        }

        traverse_all_paths_recursive(boxes, last_value, full_path, all_paths);
    }
}

#[test]
fn test_generate_all_paths() {
    let mut all_paths: Vec<Vec<i32>> = Vec::new();

    for &start in STARTING_POINTS {
        traverse_all_paths_recursive(&BOXES, start, vec![start], &mut all_paths);
    }

    let mut file = File::create("paths_output.txt").expect("Failed to create file");

    writeln!(file, "Total paths: {}\n", all_paths.len()).unwrap();

    for (i, path) in all_paths.iter().enumerate() {
        let path_str: Vec<String> = path.iter().map(|v| v.to_string()).collect();
        writeln!(file, "Path {}: [{}]", i + 1, path_str.join(", ")).unwrap();
    }

    println!("Wrote {} paths to paths_output.txt", all_paths.len());
}

#[test]
fn test_patterns_load() {
    assert!(!BOXES.is_empty(), "BOXES should not be empty");
    assert!(!STARTING_POINTS.is_empty(), "STARTING_POINTS should not be empty");
    println!("BOXES has {} keys", BOXES.len());
    println!("STARTING_POINTS has {} entries", STARTING_POINTS.len());
}

