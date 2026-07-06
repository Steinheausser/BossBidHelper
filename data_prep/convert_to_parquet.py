import os
import glob
import pandas as pd
import re

def clean_excel(file_path):
    print(f"Processing {file_path}...")
    try:
        # Determine engine based on extension
        engine = 'xlrd' if file_path.endswith('.xls') else 'openpyxl'
        df = pd.read_excel(file_path, engine=engine)
    except Exception as e:
        print(f"Error reading {file_path}: {e}")
        return None
        
    if 'Term' not in df.columns:
        print(f"File {file_path} does not have a 'Term' column. Skipping.")
        return None

    # Filter out debris rows
    df = df[df['Term'].notna()]
    df = df[~df['Term'].astype(str).str.contains('Page size', na=False)]
    
    # Rename columns if needed
    if "Course" in df.columns:
        rename_map = {
            "Course": "Course Code",
            "Sect": "Section",
            "Open": "Opening Vacancy",
            "Bef Proc": "Before Process Vacancy",
            "DICE": "D.I.C.E",
            "Aft Proc": "After Process Vacancy",
            "Enrolled": "Enrolled Students",
            "Median": "Median Bid",
            "Min": "Min Bid",
            "School": "School/Department"
        }
        df.rename(columns=rename_map, inplace=True)
        
    # We want these specific columns
    target_columns = [
        'Term', 'Session', 'Bidding Window', 'Course Code', 'Description', 
        'Section', 'Vacancy', 'Opening Vacancy', 'Before Process Vacancy', 
        'D.I.C.E', 'After Process Vacancy', 'Enrolled Students', 
        'Median Bid', 'Min Bid', 'Instructor', 'School/Department'
    ]
    
    if not all(col in df.columns for col in target_columns):
        print(f"Missing some target columns in {file_path}. Available: {df.columns.tolist()}")
        return None
        
    df = df[target_columns]
    
    # Filter Sessions: Keep 1A Session, 1B Session, Regular Academic Session (allow toggles in UI later)
    # Exclude FT Workshop Session, etc., or just keep everything and let Rust filter.
    # The instructions say "Those should be included by default. Let the user toggle them off."
    # So we'll keep everything.
    
    # Filter Rounds: Exclude incoming freshmen/exchange rounds
    df = df[~df['Bidding Window'].astype(str).str.contains('Freshmen', na=False)]
    df = df[~df['Bidding Window'].astype(str).str.contains('Exchange', na=False)]

    # Parse Round and Window
    def parse_round(window_str):
        if not isinstance(window_str, str):
            return None, None
            
        m = re.search(r'(Round\s*[12][A-Z]?).*Window\s*(\d+)', window_str, re.IGNORECASE)
        if m:
            r = m.group(1).replace(' ', '') # e.g. "Round1A"
            w = int(m.group(2))
            return r, w
        return None, None

    # Apply parsing
    parsed = df['Bidding Window'].apply(parse_round)
    df['RoundKind'] = parsed.apply(lambda x: x[0] if x[0] else "Unknown")
    df['WindowNum'] = parsed.apply(lambda x: x[1] if x[1] is not None else 0)
    
    # Exclude Unknown rounds
    df = df[df['RoundKind'] != "Unknown"]

    # Extract Year and Term Num from Term string (e.g. "2025-26 Term 1")
    def parse_term(term_str):
        if not isinstance(term_str, str):
            return 0, 0
        m = re.search(r'(\d{4})-\d{2}\s*Term\s*(\d+)', term_str, re.IGNORECASE)
        if m:
            return int(m.group(1)), int(m.group(2))
        return 0, 0
        
    parsed_term = df['Term'].apply(parse_term)
    df['Year'] = parsed_term.apply(lambda x: x[0])
    df['TermNum'] = parsed_term.apply(lambda x: x[1])

    # Clean Instructor names (trim spaces)
    df['Instructor'] = df['Instructor'].astype(str).str.strip()
    
    # Fill numeric NaNs
    numeric_cols = ['Vacancy', 'Opening Vacancy', 'Before Process Vacancy', 'D.I.C.E', 
                    'After Process Vacancy', 'Enrolled Students', 'Median Bid', 'Min Bid']
    for c in numeric_cols:
        df[c] = pd.to_numeric(df[c], errors='coerce').fillna(0)
    
    return df

def main():
    os.makedirs('data', exist_ok=True)
    all_files = glob.glob('Bid History/*.xls') + glob.glob('Bid History/*.xlsx')
    
    for f in all_files:
        df = clean_excel(f)
        if df is not None and not df.empty:
            basename = os.path.basename(f)
            out_name = os.path.splitext(basename)[0] + '.parquet'
            out_path = os.path.join('data', out_name)
            df.to_parquet(out_path, index=False)
            print(f"Saved {out_path} with {len(df)} rows.")

if __name__ == '__main__':
    main()
