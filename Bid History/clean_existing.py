import pandas as pd

def clean_excel(file_path):
    print(f"Attempting to clean {file_path}...")
    try:
        df = pd.read_excel(file_path)
    except FileNotFoundError:
        print(f"File {file_path} not found. Skipping.")
        return
        
    # Check if the file needs cleaning by looking for the "Term" column
    if 'Term' not in df.columns:
        print(f"File {file_path} does not have a 'Term' column. It might already be fully formatted or different. Skipping.")
        return

    # Filter out debris rows where 'Term' is missing or contains 'Page size'
    df = df[df['Term'].notna()]
    df = df[~df['Term'].astype(str).str.contains('Page size', na=False)]
    
    # Rename columns if it has the raw web scraped names
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
        
    # Reorder columns to the standard format
    target_columns = [
        'Term', 'Session', 'Bidding Window', 'Course Code', 'Description', 
        'Section', 'Vacancy', 'Opening Vacancy', 'Before Process Vacancy', 
        'D.I.C.E', 'After Process Vacancy', 'Enrolled Students', 
        'Median Bid', 'Min Bid', 'Instructor', 'School/Department'
    ]
    
    if all(col in df.columns for col in target_columns):
        df = df[target_columns]
    
    df.to_excel(file_path, index=False)
    print(f"Successfully cleaned and reformatted {file_path}")

# Run for both T1 and T2 just in case!
clean_excel('2025-26_T2.xlsx')
clean_excel('2025-26_T1.xlsx')
